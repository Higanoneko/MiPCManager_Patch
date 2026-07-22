//! 小米电脑管家 / 小米互联 功能增强补丁工具（命令行前端）。
//!
//! 子命令：
//!   status            查看安装信息与补丁状态
//!   locale apply|revert   地区伪装（micont_rtm.dll + 注册表）
//!   camera apply|revert   抑制「请确认摄像头状态」弹窗（PcControlCenter.dll）
//!   audio  apply|revert   音频流转无线/有线广播模式（含双网卡媒体路由修复）
//!   device apply|revert   设备伪装（释放 msimg32.dll + 注册表机型）
//!   smbios apply|revert   [实验性] SMBIOS 设备身份伪装（micont_rtm.dll IAT Hook）
//!   install           安装小米电脑管家 / 小米互联（搜索/下载安装包并按产品处理）
//!
//! 直接双击运行（无参数）时进入 ratatui TUI 全屏界面。release exe 内嵌管理员权限 manifest，启动即触发 UAC。
//! 所有补丁逻辑集中在 `mipcmanager_patch` 库中，CLI 与 GUI 共用。

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use mipcmanager_patch::{
    elevate, ops,
    experimental::{audio_dual_nic, smbios_spoof},
    install::pc_manager_installer,
    patches::device as device_spoof,
};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "MiPCM_CLI", about = "小米电脑管家功能增强补丁工具", version)]
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
    /// [实验性] SMBIOS 设备身份伪装 — 妙播 / Lyra 特殊适配（micont_rtm.dll IAT Hook）
    Smbios {
        #[command(subcommand)]
        action: SmbiosAction,
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
        /// 不自动管理 Wi-Fi 本地子网优先路由（无线模式下用于修复双网卡同网段断流）
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
    /// [实验性] 双网卡同网段音频修复：诊断并自动对齐 Wi-Fi 路由与广播模式
    ExperimentalFix {
        /// 仅诊断，不执行修复
        #[arg(long)]
        dry_run: bool,
        /// 指定版本目录（默认自动探测）
        #[arg(long)]
        dir: Option<PathBuf>,
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

#[derive(Subcommand, Clone)]
enum SmbiosAction {
    /// 应用 SMBIOS 设备身份伪装
    Apply {
        /// 机型代号（默认 TM2424）
        #[arg(long, default_value = smbios_spoof::DEFAULT_MODEL)]
        model: String,
        /// 指定目标 DLL 路径（默认自动探测安装目录）
        #[arg(long)]
        dll: Option<PathBuf>,
        #[arg(long)]
        no_kill: bool,
    },
    /// 还原 SMBIOS 设备身份伪装
    Revert {
        #[arg(long)]
        dll: Option<PathBuf>,
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
        None => mipcmanager_patch::ui::tui::run(),
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
            AudioAction::ExperimentalFix { dry_run, dir } => {
                let dir = dir.map(Ok).unwrap_or_else(ops::resolve_full_version_dir)?;
                if dry_run {
                    print_log(audio_dual_nic::diagnose(&dir)?);
                } else {
                    print_log(audio_dual_nic::auto_fix(&dir)?);
                }
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
        Command::Smbios { action } => match action {
            SmbiosAction::Apply {
                model,
                dll,
                no_kill,
            } => {
                print_log(ops::apply_smbios(Some(&model), dll, no_kill)?);
                Ok(())
            }
            SmbiosAction::Revert { dll, no_kill } => {
                print_log(ops::revert_smbios(dll, no_kill)?);
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

// ── 数据驱动的安装包选择菜单 ────────────────────────────────────

/// 菜单动作：定义每个菜单项对应的业务逻辑。
/// 新增菜单项时在此添加变体并在 [`INSTALLER_SOURCE_MENU`] 中注册。
enum InstallerSourceAction {
    DownloadUrl,
    SpecifyPath,
}

/// 声明式菜单项：key 匹配用户输入，description 为显示文本，action 为对应逻辑。
struct MenuEntry {
    key: &'static str,
    description: &'static str,
    action: InstallerSourceAction,
}

/// 安装包来源选择菜单 — 新增选项只需在此切片追加一条记录。
const INSTALLER_SOURCE_MENU: &[MenuEntry] = &[
    MenuEntry {
        key: "1",
        description: "输入下载网址",
        action: InstallerSourceAction::DownloadUrl,
    },
    MenuEntry {
        key: "2",
        description: "指定本地 .exe 安装包",
        action: InstallerSourceAction::SpecifyPath,
    },
];

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
    for entry in INSTALLER_SOURCE_MENU {
        println!("  {}) {}", entry.key, entry.description);
    }
    println!("  0) 取消");
    let choice = prompt("请选择：")?;
    let action = INSTALLER_SOURCE_MENU
        .iter()
        .find(|e| e.key == choice)
        .map(|e| &e.action);
    match action {
        Some(InstallerSourceAction::DownloadUrl) => {
            let url = prompt("请输入 HTTP(S) 下载地址：")?;
            if url.is_empty() {
                return Ok(None);
            }
            println!("正在下载安装包到 {}", patcher_dir.display());
            pc_manager_installer::download_installer(&url, patcher_dir).map(Some)
        }
        Some(InstallerSourceAction::SpecifyPath) => {
            let input = prompt("请输入 .exe 安装包路径：")?;
            let path = input.trim().trim_matches(['"', '\'']);
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(path)))
            }
        }
        None if choice == "0" || choice.is_empty() => Ok(None),
        None => bail!("无效选择"),
    }
}

fn prompt(msg: &str) -> Result<String> {
    use std::io::Write;
    print!("{msg}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod install_routing_tests {
    use super::*;

    #[test]
    fn install_cli_accepts_exactly_one_package_source() {
        assert!(Cli::try_parse_from(["MiPCM_CLI", "install"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "MiPCM_CLI",
                "install",
                "--installer",
                "XiaomiPCManager.exe"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "MiPCM_CLI",
                "install",
                "--url",
                "https://example.com/XiaomiPCManager.exe"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "MiPCM_CLI",
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
