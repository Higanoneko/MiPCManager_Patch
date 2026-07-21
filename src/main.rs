//! 小米电脑管家 / 小米互联 功能增强补丁工具（命令行前端）。
//!
//! 子命令：
//!   status            查看安装信息与补丁状态
//!   locale apply|revert   地区伪装（micont_rtm.dll + 注册表）
//!   camera apply|revert   抑制「请确认摄像头状态」弹窗（PcControlCenter.dll）
//!   audio  apply|revert   音频流转无线/有线广播模式（含双网卡媒体路由修复）
//!   device apply|revert   设备伪装（释放 msimg32.dll + 注册表机型）
//!   install           安装小米电脑管家 / 小米互联（搜索/下载安装包并按产品处理）
//!
//! 直接双击运行（无参数）时进入交互菜单。release exe 内嵌管理员权限 manifest，启动即触发 UAC。
//! 所有补丁逻辑集中在 `mipcmanager_patch` 库中，CLI 与 GUI 共用。

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use mipcmanager_patch::{device_spoof, elevate, ops, pc_manager_installer};
use std::io::{Write, stdin, stdout};
use std::path::{Path, PathBuf};

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
    /// MiPCAudio 音频流转广播模式（无线/有线，统一身份并修复双网卡媒体路由）
    Audio {
        #[command(subcommand)]
        action: AudioAction,
    },
    /// 设备伪装（释放 msimg32.dll + 写入机型注册表）
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
    /// 安装小米电脑管家 / 小米互联（自动识别安装包所属产品）
    Install {
        /// 显式指定 .exe 安装包
        #[arg(long, value_name = "EXE", conflicts_with = "url")]
        installer: Option<PathBuf>,
        /// 从 HTTP(S) 地址下载安装包
        #[arg(long, value_name = "URL", conflicts_with = "installer")]
        url: Option<String>,
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
        /// 有线广播时，不创建 Wi-Fi 本地子网路由（双网卡同网段时可能导致手机立即断流）
        #[arg(long)]
        no_wifi_local_route: bool,
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

impl From<ModeArg> for ops::BroadcastMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Wifi => ops::BroadcastMode::Wireless,
            ModeArg::Lan => ops::BroadcastMode::Wired,
        }
    }
}

fn main() {
    // Release exe 通过 manifest 在启动前请求管理员权限；这里保留运行时兜底。
    elevate::ensure_elevated();
    if let Some(message) = ops::close_all_on_startup() {
        println!("{message}");
    }

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
        Command::Status => {
            print_log(ops::status_lines());
            Ok(())
        }
        Command::Locale {
            action,
            region,
            no_registry,
        } => match action {
            PatchAction::Apply { dll, no_kill } => {
                print_log(ops::apply_locale(dll, &region, !no_registry, no_kill)?);
                Ok(())
            }
            PatchAction::Revert { dll, no_kill } => {
                print_log(ops::revert_locale(dll, !no_registry, no_kill)?);
                Ok(())
            }
        },
        Command::Camera { action } => match action {
            PatchAction::Apply { dll, no_kill } => {
                print_log(ops::apply_camera(dll, no_kill)?);
                Ok(())
            }
            PatchAction::Revert { dll, no_kill } => {
                print_log(ops::revert_camera(dll, no_kill)?);
                Ok(())
            }
        },
        Command::Audio { action } => match action {
            AudioAction::Apply {
                mode,
                dir,
                no_kill,
                no_wifi_local_route,
            } => {
                print_log(ops::apply_audio_with_options(
                    mode.into(),
                    dir,
                    no_kill,
                    no_wifi_local_route,
                )?);
                Ok(())
            }
            AudioAction::Revert { dir, no_kill } => {
                print_log(ops::revert_audio(dir, no_kill)?);
                Ok(())
            }
        },
        Command::Device { action } => match action {
            DeviceAction::Apply {
                model,
                dir,
                no_kill,
            } => {
                print_log(ops::apply_device(&model, dir, no_kill)?);
                Ok(())
            }
            DeviceAction::Revert { dir, no_kill } => {
                print_log(ops::revert_device(dir, no_kill)?);
                Ok(())
            }
        },
        Command::Install { installer, url } => install_pc_manager(installer, url),
    }
}

fn print_log(lines: Vec<String>) {
    for line in lines {
        println!("{line}");
    }
}

// ===================== 安装（交互式选择安装包） =====================

fn install_pc_manager(explicit: Option<PathBuf>, url: Option<String>) -> Result<()> {
    let patcher_dir = pc_manager_installer::patcher_dir()?;
    let Some(installer) = choose_manager_installer(explicit, url.as_deref(), &patcher_dir)? else {
        println!("已取消安装。");
        return Ok(());
    };
    print_log(ops::install_from_path(&installer)?);
    Ok(())
}

fn choose_manager_installer(
    explicit: Option<PathBuf>,
    url: Option<&str>,
    patcher_dir: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(path));
    }
    if let Some(url) = url {
        println!("正在下载安装包到 {}", patcher_dir.display());
        return pc_manager_installer::download_installer(url, patcher_dir).map(Some);
    }

    let candidates = pc_manager_installer::find_local_installers(patcher_dir)?;
    match candidates.as_slice() {
        [only] => {
            let kind = pc_manager_installer::classify_installer(only);
            println!("已找到{}安装包：{}", kind.label(), only.display());
            Ok(Some(only.clone()))
        }
        [] => prompt_installer_source(patcher_dir),
        _ => prompt_installer_candidate(&candidates),
    }
}

fn prompt_installer_candidate(candidates: &[PathBuf]) -> Result<Option<PathBuf>> {
    println!("找到多个安装包：");
    for (index, path) in candidates.iter().enumerate() {
        let kind = pc_manager_installer::classify_installer(path);
        println!("  {}) [{}] {}", index + 1, kind.label(), path.display());
    }
    println!("  0) 取消");
    let choice = prompt("请选择安装包：")?;
    if choice == "0" || choice.is_empty() {
        return Ok(None);
    }
    let index = choice
        .parse::<usize>()
        .ok()
        .filter(|index| (1..=candidates.len()).contains(index))
        .context("无效的安装包选择")?;
    Ok(Some(candidates[index - 1].clone()))
}

fn prompt_installer_source(patcher_dir: &Path) -> Result<Option<PathBuf>> {
    println!(
        "未在 Patcher 同目录找到安装包（小米电脑管家 `*_XiaomiPCManager_*.exe` 或小米互联 `小米互联*.exe`）。"
    );
    println!("  1) 输入下载网址");
    println!("  2) 指定本地 .exe 安装包");
    println!("  0) 取消");
    match prompt("请选择：")?.as_str() {
        "1" => {
            let url = prompt("请输入 HTTP(S) 下载地址：")?;
            if url.is_empty() {
                return Ok(None);
            }
            println!("正在下载安装包到 {}", patcher_dir.display());
            pc_manager_installer::download_installer(&url, patcher_dir).map(Some)
        }
        "2" => {
            let input = prompt("请输入 .exe 安装包路径：")?;
            let path = input.trim().trim_matches(['"', '\'']);
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(path)))
            }
        }
        "0" | "" => Ok(None),
        _ => bail!("无效选择"),
    }
}

// ===================== 交互菜单 =====================

fn interactive_menu() -> Result<()> {
    loop {
        let full_features_available = ops::full_features_available();
        println!("\n=== 小米电脑管家增强补丁 ===");
        println!("  1) 查看状态");
        println!("  2) 地区伪装");
        let unavailable = if full_features_available {
            ""
        } else {
            "（当前安装不可用）"
        };
        println!("  3) 抑制摄像头弹窗{unavailable}");
        println!("  4) 音频流转增强{unavailable}");
        println!("  5) 设备伪装{unavailable}");
        println!("  6) 安装小米电脑管家 / 小米互联");
        println!("  0) 退出");
        match prompt("请选择：")?.as_str() {
            "1" => run_logged(run(Command::Status)),
            "2" => menu_locale(),
            "3" if full_features_available => menu_camera(),
            "4" if full_features_available => menu_audio(),
            "5" if full_features_available => menu_device(),
            "3" | "4" | "5" => println!("PcContinuity 暂时仅支持地区伪装。"),
            "6" => {
                run_logged(run(Command::Install {
                    installer: None,
                    url: None,
                }));
            }
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
            no_wifi_local_route: false,
        },
        "2" => AudioAction::Apply {
            mode: ModeArg::Lan,
            dir: None,
            no_kill: false,
            no_wifi_local_route: false,
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

#[cfg(test)]
mod install_routing_tests {
    use super::*;

    #[test]
    fn install_cli_accepts_exactly_one_package_source() {
        assert!(Cli::try_parse_from(["mipcm_patch", "install"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "mipcm_patch",
                "install",
                "--installer",
                "XiaomiPCManager.exe"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "mipcm_patch",
                "install",
                "--url",
                "https://example.com/XiaomiPCManager.exe"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "mipcm_patch",
                "install",
                "--installer",
                "local.exe",
                "--url",
                "https://example.com/remote.exe"
            ])
            .is_err()
        );
    }
}
