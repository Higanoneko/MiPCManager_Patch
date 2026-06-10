//! 小米电脑管家 (MiPCManager) 功能增强补丁工具。
//!
//! 子命令：
//!   status            查看安装信息与补丁状态
//!   locale apply|revert   地区伪装（micont_rtm.dll + 注册表）
//!   camera apply|revert   抑制「请确认摄像头状态」弹窗（PcControlCenter.dll）
//!   audio  apply|revert   音频流转无线/有线广播模式（MiPCAudio.exe + idmruntime.dll）
//!   device apply|revert   设备伪装（释放 msimg32.dll + 注册表机型）
//!
//! 直接双击运行（无参数）时进入交互菜单。release exe 内嵌管理员权限 manifest，启动即触发 UAC。

mod camera_toast;
mod device_spoof;
mod dotnet;
mod elevate;
mod install;
mod locale_spoof;
mod mipcaudio_lan;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::{Write, stdin, stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 小米电脑管家相关进程（不含扩展名），用于启动时全量关闭的兜底匹配。
const PROC_MIPCM_ALL: &[&str] = &[
    "XiaomiPcManager",
    "XiaomiPcHost",
    "micont_service",
    "MiPCAudio",
    "MiDistributedCameraBroker",
    "MiDistributedCameraBroker32",
    "MiHygieneBroker",
    "MiPlayCastService",
    "MiScreenShare",
    "MiSmartShareDevice",
    "MiSmartShareHandoff",
    "mistreamservice",
    "PcClipboard",
    "PcyybAssistant",
    "XaAppStore",
    "handoff_svc",
    "dist_service",
    "DistributedService",
    "MAFSvr",
    "MASFvr",
    "OSDLauncher",
    "OSDUtility",
    "SambaServer",
];

/// 各功能在打补丁前需要关闭的进程（不含扩展名）。补丁后由用户手动重新打开。
const PROC_LOCALE: &[&str] = &["micont_service"];
const PROC_CAMERA: &[&str] = &["XiaomiPcManager"];
const PROC_AUDIO: &[&str] = &["MiPCAudio", "MAFSvr", "MASFvr"];
const PROC_DEVICE: &[&str] = &["XiaomiPcManager"];

#[derive(Parser)]
#[command(name = "mipcm_patch", about = "小米电脑管家功能增强补丁工具", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 查看安装信息与补丁状态
    Status,
    /// 地区伪装（micont_rtm.dll）
    Locale {
        #[command(subcommand)]
        action: PatchAction,
        /// 伪装地区代码（默认 CN）
        #[arg(long, default_value = "CN", global = true)]
        region: String,
        /// 不修改注册表
        #[arg(long, global = true)]
        no_registry: bool,
    },
    /// 抑制「请确认摄像头状态」弹窗（PcControlCenter.dll）
    Camera {
        #[command(subcommand)]
        action: PatchAction,
    },
    /// MiPCAudio 音频流转广播模式（无线/有线，统一三处身份消除重复设备）
    Audio {
        #[command(subcommand)]
        action: AudioAction,
    },
    /// 设备伪装（释放 msimg32.dll + 写入机型注册表）
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
}

#[derive(Subcommand, Clone)]
enum PatchAction {
    /// 应用补丁
    Apply {
        /// 指定目标 DLL 路径（默认自动探测安装目录）
        #[arg(long)]
        dll: Option<PathBuf>,
        /// 不自动关闭相关进程
        #[arg(long)]
        no_kill: bool,
    },
    /// 还原补丁
    Revert {
        #[arg(long)]
        dll: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
}

#[derive(Subcommand, Clone)]
enum AudioAction {
    /// 切换广播模式
    Apply {
        /// 广播介质：wifi（无线，默认）或 lan（有线）
        #[arg(long, value_enum, default_value_t = ModeArg::Wifi)]
        mode: ModeArg,
        /// 指定版本目录（默认自动探测）
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
    /// 还原音频流转补丁
    Revert {
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
}

#[derive(Subcommand, Clone)]
enum DeviceAction {
    /// 应用设备伪装
    Apply {
        /// 机型代号（默认 TM2424）
        #[arg(long, default_value = device_spoof::DEFAULT_MODEL)]
        model: String,
        /// 指定版本目录（默认自动探测）
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
    /// 还原设备伪装
    Revert {
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
}

#[derive(ValueEnum, Clone, Copy)]
enum ModeArg {
    Wifi,
    Lan,
}

impl From<ModeArg> for mipcaudio_lan::BroadcastMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Wifi => mipcaudio_lan::BroadcastMode::Wireless,
            ModeArg::Lan => mipcaudio_lan::BroadcastMode::Wired,
        }
    }
}

fn main() {
    // Release exe 通过 manifest 在启动前请求管理员权限；这里保留运行时兜底。
    elevate::ensure_elevated();
    close_all_mipcmanager_apps_on_startup();

    let cli = Cli::parse();
    let result = match cli.command {
        Some(cmd) => run(cmd),
        None => interactive_menu(),
    };
    if let Err(e) = result {
        eprintln!("错误：{e:#}");
        std::process::exit(1);
    }
}

fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Status => status(),
        Command::Locale {
            action,
            region,
            no_registry,
        } => match action {
            PatchAction::Apply { dll, no_kill } => {
                let path = resolve_dll(dll, locale_spoof::TARGET_DLL)?;
                close_apps_required(PROC_LOCALE, no_kill)?;
                let out = retry_patch_after_access_denied(PROC_LOCALE, || {
                    locale_spoof::apply(&path, &region, !no_registry)
                })?;
                report_locale(out, &path, &region, !no_registry);
                remind_restart();
                Ok(())
            }
            PatchAction::Revert { dll, no_kill } => {
                let path = resolve_dll(dll, locale_spoof::TARGET_DLL)?;
                close_apps_required(PROC_LOCALE, no_kill)?;
                retry_patch_after_access_denied(PROC_LOCALE, || {
                    locale_spoof::revert(&path, !no_registry)
                })?;
                println!("✓ 已还原地区伪装：{}", path.display());
                remind_restart();
                Ok(())
            }
        },
        Command::Camera { action } => match action {
            PatchAction::Apply { dll, no_kill } => {
                let path = resolve_dll(dll, camera_toast::TARGET_DLL)?;
                close_apps(PROC_CAMERA, no_kill);
                let out =
                    retry_patch_after_access_denied(PROC_CAMERA, || camera_toast::apply(&path))?;
                report_inject(out, &path);
                remind_restart();
                Ok(())
            }
            PatchAction::Revert { dll, no_kill } => {
                let path = resolve_dll(dll, camera_toast::TARGET_DLL)?;
                close_apps(PROC_CAMERA, no_kill);
                retry_patch_after_access_denied(PROC_CAMERA, || camera_toast::revert(&path))?;
                println!("✓ 已还原摄像头弹窗补丁：{}", path.display());
                remind_restart();
                Ok(())
            }
        },
        Command::Audio { action } => match action {
            AudioAction::Apply { mode, dir, no_kill } => {
                let dir = resolve_version_dir_or(dir)?;
                close_apps(PROC_AUDIO, no_kill);
                let mode: mipcaudio_lan::BroadcastMode = mode.into();
                let results = retry_patch_after_access_denied(PROC_AUDIO, || {
                    mipcaudio_lan::apply(&dir, mode)
                })?;
                println!("✓ 音频流转广播模式已切换为：{}", mode.label());
                for (file, o) in results {
                    println!("  {file}: 改写 {} 处, 已是目标 {} 处", o.patched, o.already);
                }
                println!("  （三处网卡身份已统一 → 手机端应为单设备）");
                remind_restart();
                Ok(())
            }
            AudioAction::Revert { dir, no_kill } => {
                let dir = resolve_version_dir_or(dir)?;
                close_apps(PROC_AUDIO, no_kill);
                retry_patch_after_access_denied(PROC_AUDIO, || mipcaudio_lan::revert(&dir))?;
                println!("✓ 已还原音频流转补丁（MiPCAudio.exe / idmruntime.dll）");
                remind_restart();
                Ok(())
            }
        },
        Command::Device { action } => match action {
            DeviceAction::Apply {
                model,
                dir,
                no_kill,
            } => {
                let dir = resolve_version_dir_or(dir)?;
                close_apps(PROC_DEVICE, no_kill);
                retry_patch_after_access_denied(PROC_DEVICE, || device_spoof::apply(&dir, &model))?;
                println!("✓ 设备伪装已应用：机型 = {model}");
                println!(
                    "  已释放 {} 至 {}",
                    device_spoof::PROXY_DLL_NAME,
                    dir.display()
                );
                println!("  注册表 HKCU\\Software\\SmartSharePatch\\SpoofDevice = {model}");
                remind_restart();
                Ok(())
            }
            DeviceAction::Revert { dir, no_kill } => {
                let dir = resolve_version_dir_or(dir)?;
                close_apps(PROC_DEVICE, no_kill);
                retry_patch_after_access_denied(PROC_DEVICE, || device_spoof::revert(&dir))?;
                println!("✓ 已还原设备伪装（移除 msimg32.dll 与注册表机型）");
                remind_restart();
                Ok(())
            }
        },
    }
}

/// 解析单个目标 DLL 路径：优先显式路径，否则探测安装目录最新版本。
fn resolve_dll(explicit: Option<PathBuf>, filename: &str) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.exists() {
            bail!("指定的文件不存在：{}", p.display());
        }
        return Ok(p);
    }
    let ver = resolve_version_dir()?;
    let path = ver.join(filename);
    if !path.exists() {
        bail!("在 {} 中未找到 {filename}", ver.display());
    }
    Ok(path)
}

/// 探测安装目录的最新版本目录。
fn resolve_version_dir() -> Result<PathBuf> {
    let root = install::find_install_root().context("未找到小米电脑管家安装目录")?;
    install::latest_version_dir(&root)
}

/// 显式版本目录优先，否则自动探测。
fn resolve_version_dir_or(explicit: Option<PathBuf>) -> Result<PathBuf> {
    match explicit {
        Some(d) if d.is_dir() => Ok(d),
        Some(d) => bail!("指定的版本目录不存在：{}", d.display()),
        None => resolve_version_dir(),
    }
}

/// 打补丁前关闭相关进程。
fn close_apps(procs: &[&str], no_kill: bool) {
    if no_kill {
        return;
    }
    let n = install::kill_by_names(procs);
    if n > 0 {
        println!("已关闭 {n} 个相关进程：{}", procs.join(", "));
    }
}

/// 需要确保进程已经退出的补丁操作使用该函数；若仍在运行则拒绝继续 Patch。
fn close_apps_required(procs: &[&str], no_kill: bool) -> Result<()> {
    let (n, running) = if no_kill {
        (0, install::running_by_names(procs))
    } else {
        install::kill_by_names_until_gone(procs, Duration::from_secs(5))
    };
    if n > 0 {
        println!("已关闭 {n} 个相关进程：{}", procs.join(", "));
    }
    if !running.is_empty() {
        bail!(
            "补丁前必须关闭相关进程，但以下进程仍在运行：{}。请手动结束后重试。",
            running.join(", ")
        );
    }
    Ok(())
}

/// 文件被进程占用时，Windows 常返回 access denied；此时关闭对应进程并重试一次。
fn retry_patch_after_access_denied<T, F>(procs: &[&str], mut patch: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    match patch() {
        Ok(v) => Ok(v),
        Err(e) if is_access_denied(&e) => {
            println!(
                "遇到拒绝访问，正在关闭相关进程后重试一次：{}",
                procs.join(", ")
            );
            let (n, running) = install::kill_by_names_until_gone(procs, Duration::from_secs(5));
            if n > 0 {
                println!("已关闭 {n} 个相关进程：{}", procs.join(", "));
            }
            if !running.is_empty() {
                println!("以下进程仍在运行，将按要求重试一次：{}", running.join(", "));
            }
            patch()
        }
        Err(e) => Err(e),
    }
}

fn is_access_denied(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            io.kind() == std::io::ErrorKind::PermissionDenied || io.raw_os_error() == Some(5)
        })
    })
}

/// 启动时关闭所有小米电脑管家相关进程，避免文件占用或后台服务立刻拉起组件。
fn close_all_mipcmanager_apps_on_startup() {
    let n = install::kill_mipcmanager_processes(PROC_MIPCM_ALL);
    if n > 0 {
        println!("已关闭 {n} 个小米电脑管家相关进程。");
    }
}

fn remind_restart() {
    println!("提示：补丁已完成，请手动重新启动小米电脑管家使其生效。");
}

fn report_locale(out: locale_spoof::PatchOutcome, path: &Path, region: &str, reg: bool) {
    match out {
        locale_spoof::PatchOutcome::Patched => {
            println!("✓ 地区伪装已应用：{}", path.display());
        }
        locale_spoof::PatchOutcome::AlreadyPatched => {
            println!("• DLL 已是补丁状态（跳过）：{}", path.display());
        }
    }
    if reg {
        println!("  注册表 HKCU\\Control Panel\\International\\Geo\\XCN = {region}");
    }
}

fn report_inject(out: dotnet::InjectOutcome, path: &Path) {
    match out {
        dotnet::InjectOutcome::Patched => {
            println!("✓ 摄像头弹窗补丁已应用：{}", path.display());
        }
        dotnet::InjectOutcome::AlreadyPatched => {
            println!("• 已是补丁状态（跳过）：{}", path.display());
        }
    }
}

fn status() -> Result<()> {
    println!("== 小米电脑管家补丁状态 ==");
    match install::find_install_root() {
        Some(root) => {
            println!("安装根目录：{}", root.display());
            match install::latest_version_dir(&root) {
                Ok(ver) => {
                    println!("最新版本目录：{}", ver.display());
                    print_file_status(&ver.join(locale_spoof::TARGET_DLL));
                    print_file_status(&ver.join(camera_toast::TARGET_DLL));
                    println!("  -- 音频流转广播模式 --");
                    for (file, state) in mipcaudio_lan::current_state(&ver) {
                        println!("     {file}: {state}");
                    }
                    println!("  -- 设备伪装 --");
                    let (dll_ok, model) = device_spoof::current_state(&ver);
                    println!(
                        "     msimg32.dll: {} | 注册表机型: {}",
                        if dll_ok { "已就位" } else { "未就位" },
                        model.unwrap_or_else(|| "未设置".to_string())
                    );
                }
                Err(e) => println!("（无法确定版本目录：{e}）"),
            }
        }
        None => println!("未探测到安装目录（可用 --dll/--dir 手动指定）。"),
    }
    Ok(())
}

fn print_file_status(path: &Path) {
    let exists = path.exists();
    let bak = install::backup_path(path).exists();
    println!(
        "  {} | 存在:{} | 备份:{}",
        path.file_name().unwrap().to_string_lossy(),
        if exists { "是" } else { "否" },
        if bak { "有" } else { "无" }
    );
}

// ===================== 交互菜单 =====================

fn interactive_menu() -> Result<()> {
    loop {
        println!("\n=== 小米电脑管家增强补丁 ===");
        println!("  1) 查看状态");
        println!("  2) 地区伪装");
        println!("  3) 抑制摄像头弹窗");
        println!("  4) 音频流转增强");
        println!("  5) 设备伪装");
        println!("  0) 退出");
        match prompt("请选择：")?.as_str() {
            "1" => run_logged(status()),
            "2" => menu_locale(),
            "3" => menu_camera(),
            "4" => menu_audio(),
            "5" => menu_device(),
            "0" | "q" | "exit" => break,
            _ => println!("无效选择。"),
        }
    }
    Ok(())
}

fn menu_locale() {
    println!("\n-- 地区伪装 --");
    println!("  1) 应用    2) 还原    0) 返回");
    match read_choice() {
        "1" => run_logged(run(Command::Locale {
            action: PatchAction::Apply {
                dll: None,
                no_kill: false,
            },
            region: "CN".into(),
            no_registry: false,
        })),
        "2" => run_logged(run(Command::Locale {
            action: PatchAction::Revert {
                dll: None,
                no_kill: false,
            },
            region: "CN".into(),
            no_registry: false,
        })),
        _ => {}
    }
}

fn menu_camera() {
    println!("\n-- 抑制摄像头弹窗 --");
    println!("  1) 应用    2) 还原    0) 返回");
    match read_choice() {
        "1" => run_logged(run(Command::Camera {
            action: PatchAction::Apply {
                dll: None,
                no_kill: false,
            },
        })),
        "2" => run_logged(run(Command::Camera {
            action: PatchAction::Revert {
                dll: None,
                no_kill: false,
            },
        })),
        _ => {}
    }
}

fn menu_audio() {
    println!("\n-- 音频流转增强--");
    println!("  1) 切换为【无线 WiFi】广播");
    println!("  2) 切换为【有线 LAN】广播");
    println!("  3) 还原");
    println!("  0) 返回");
    let action = match read_choice() {
        "1" => AudioAction::Apply {
            mode: ModeArg::Wifi,
            dir: None,
            no_kill: false,
        },
        "2" => AudioAction::Apply {
            mode: ModeArg::Lan,
            dir: None,
            no_kill: false,
        },
        "3" => AudioAction::Revert {
            dir: None,
            no_kill: false,
        },
        _ => return,
    };
    run_logged(run(Command::Audio { action }));
}

fn menu_device() {
    println!("\n-- 设备伪装 --");
    println!("  1) 应用    2) 还原    0) 返回");
    match read_choice() {
        "1" => {
            let model = match choose_model() {
                Some(m) => m,
                None => return,
            };
            run_logged(run(Command::Device {
                action: DeviceAction::Apply {
                    model,
                    dir: None,
                    no_kill: false,
                },
            }));
        }
        "2" => run_logged(run(Command::Device {
            action: DeviceAction::Revert {
                dir: None,
                no_kill: false,
            },
        })),
        _ => {}
    }
}

/// 让用户选择伪装机型，返回机型代号。
fn choose_model() -> Option<String> {
    println!("\n选择伪装机型：");
    for (i, p) in device_spoof::PRESETS.iter().enumerate() {
        let tag = if p.code == device_spoof::DEFAULT_MODEL {
            "（默认）"
        } else {
            ""
        };
        println!("  {}) {} [{}]{}", i + 1, p.code, p.name, tag);
    }
    println!("  c) 自定义机型代号");
    println!("  0) 返回");
    let c = read_choice();
    match c {
        "0" => None,
        "c" | "C" => {
            let m = prompt("请输入机型代号：").ok()?;
            if m.trim().is_empty() {
                println!("机型代号为空，已取消。");
                None
            } else {
                Some(m.trim().to_string())
            }
        }
        other => match other.parse::<usize>() {
            Ok(n) if n >= 1 && n <= device_spoof::PRESETS.len() => {
                Some(device_spoof::PRESETS[n - 1].code.to_string())
            }
            _ => {
                println!("无效选择。");
                None
            }
        },
    }
}

/// 打印提示并读取一行输入（去除首尾空白）。
fn prompt(msg: &str) -> Result<String> {
    print!("{msg}");
    stdout().flush().ok();
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// 读取菜单选择；读取失败时返回 "0"（视作返回）。
fn read_choice() -> &'static str {
    let s = prompt("请选择：").unwrap_or_default();
    // 将常见输入映射为静态字符串，便于 match。
    match s.as_str() {
        "1" => "1",
        "2" => "2",
        "3" => "3",
        "c" | "C" => "c",
        "0" | "q" | "exit" | "" => "0",
        _ => "?",
    }
}

/// 执行一个操作并在出错时打印红色错误（用于菜单，不中断循环）。
fn run_logged(r: Result<()>) {
    if let Err(e) = r {
        eprintln!("错误：{e:#}");
    }
}
