//! 小米电脑管家 / 小米互联 功能增强补丁工具（命令行前端）。

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use mipcmanager_patch::{
    elevate, i18n, ops,
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
    /// 卸载 MiDrop Ext MSIX 包 或 小米电脑管家 / 小米互联
    Uninstall {
        #[command(subcommand)]
        action: UninstallAction,
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

#[derive(Subcommand, Clone)]
enum UninstallAction {
    /// 卸载 MiDrop Ext MSIX 包（完成后重启资源管理器）
    Msix,
    /// 完整卸载小米电脑管家 / 小米互联（主程序 + 子产品 + 服务 + 文件清理，不可逆）
    Product,
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
    elevate::ensure_elevated();
    let lang = i18n::detect_lang();
    if let Some(message) = ops::close_all_on_startup() {
        println!("{message}");
    }

    let cli = Cli::parse();
    let result = match cli.command {
        Some(cmd) => run(cmd, lang),
        None => mipcmanager_patch::ui::tui::run(),
    };
    if let Err(e) = result {
        eprintln!("{}", i18n::tr("cli.error", lang).replace("{error}", &format!("{e:#}")));
        std::process::exit(1);
    }
}

fn run(cmd: Command, lang: i18n::Lang) -> Result<()> {
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
        Command::Install { installer, url } => install_pc_manager(installer, url, lang),
        Command::Uninstall { action } => match action {
            UninstallAction::Msix => {
                print_log(ops::uninstall_msix(false)?);
                Ok(())
            }
            UninstallAction::Product => {
                let desc = ops::uninstall_product_description()?;
                println!("{desc}");
                println!();
                let input = prompt(i18n::tr("cli.confirm.uninstall", lang))?;
                if input.to_lowercase() != "y" {
                    println!("{}", i18n::tr("cli.cancelled.uninstall", lang));
                    return Ok(());
                }
                print_log(ops::uninstall_product()?);
                Ok(())
            }
        },
    }
}

fn print_log(lines: Vec<String>) {
    for line in lines {
        println!("{line}");
    }
}

// ===================== 安装（交互式选择安装包） =====================

fn install_pc_manager(explicit: Option<PathBuf>, url: Option<String>, lang: i18n::Lang) -> Result<()> {
    let patcher_dir = pc_manager_installer::patcher_dir()?;
    let Some(installer) = choose_manager_installer(explicit, url.as_deref(), &patcher_dir, lang)? else {
        println!("{}", i18n::tr("cli.cancelled.install", lang));
        return Ok(());
    };
    print_log(ops::install_from_path(&installer)?);
    Ok(())
}

fn choose_manager_installer(
    explicit: Option<PathBuf>,
    url: Option<&str>,
    patcher_dir: &Path,
    lang: i18n::Lang,
) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(path));
    }
    if let Some(url) = url {
        println!("{}", i18n::tr("cli.downloading", lang).replace("{path}", &patcher_dir.display().to_string()));
        return pc_manager_installer::download_installer(url, patcher_dir).map(Some);
    }

    let candidates = pc_manager_installer::find_local_installers(patcher_dir)?;
    match candidates.as_slice() {
        [only] => {
            let kind = pc_manager_installer::classify_installer(only);
            println!("{}", i18n::tr("cli.found.installer", lang)
                .replace("{kind}", kind.label_for(lang))
                .replace("{path}", &only.display().to_string()));
            Ok(Some(only.clone()))
        }
        [] => prompt_installer_source(patcher_dir, lang),
        _ => prompt_installer_candidate(&candidates, lang),
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
        description: "cli.menu.download_url",
        action: InstallerSourceAction::DownloadUrl,
    },
    MenuEntry {
        key: "2",
        description: "cli.menu.local_file",
        action: InstallerSourceAction::SpecifyPath,
    },
];

fn prompt_installer_candidate(candidates: &[PathBuf], lang: i18n::Lang) -> Result<Option<PathBuf>> {
    println!("{}", i18n::tr("cli.multiple.installers", lang));
    for (index, path) in candidates.iter().enumerate() {
        let kind = pc_manager_installer::classify_installer(path);
        println!("  {}) [{}] {}", index + 1, kind.label_for(lang), path.display());
    }
    println!("  0) {}", i18n::tr("cli.cancel.option", lang));
    let choice = prompt(i18n::tr("cli.choose.installer", lang))?;
    if choice == "0" || choice.is_empty() {
        return Ok(None);
    }
    let index = choice
        .parse::<usize>()
        .ok()
        .filter(|index| (1..=candidates.len()).contains(index))
        .context(i18n::tr("cli.invalid.choice", lang))?;
    Ok(Some(candidates[index - 1].clone()))
}

fn prompt_installer_source(patcher_dir: &Path, lang: i18n::Lang) -> Result<Option<PathBuf>> {
    println!("{}", i18n::tr("cli.no.installer.found", lang));
    for entry in INSTALLER_SOURCE_MENU {
        println!("  {}) {}", entry.key, i18n::tr(entry.description, lang));
    }
    println!("  0) {}", i18n::tr("cli.cancel.option", lang));
    let choice = prompt(i18n::tr("cli.choose.option", lang))?;
    let action = INSTALLER_SOURCE_MENU
        .iter()
        .find(|e| e.key == choice)
        .map(|e| &e.action);
    match action {
        Some(InstallerSourceAction::DownloadUrl) => {
            let url = prompt(i18n::tr("cli.prompt.url", lang))?;
            if url.is_empty() {
                return Ok(None);
            }
            println!("{}", i18n::tr("cli.downloading", lang).replace("{path}", &patcher_dir.display().to_string()));
            pc_manager_installer::download_installer(&url, patcher_dir).map(Some)
        }
        Some(InstallerSourceAction::SpecifyPath) => {
            let input = prompt(i18n::tr("cli.prompt.path", lang))?;
            let path = input.trim().trim_matches(['"', '\'']);
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(path)))
            }
        }
        None if choice == "0" || choice.is_empty() => Ok(None),
        None => bail!(i18n::tr("cli.invalid.selection", lang)),
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
