//! 面向前端（CLI / GUI）的高层操作。
//!
//! 所有补丁动作都在这里编排：解析目标路径 → 关闭相关进程 → 应用/还原补丁 → 汇总日志。
//! 每个高层函数返回 `Vec<String>` 日志行，CLI 直接打印、GUI 追加到日志区，二者共用同一套逻辑。

use crate::{
    audio_wifi_route, camera_toast, device_spoof, dotnet, install, locale_spoof, mipcaudio_lan,
    pc_manager_installer, smbios_spoof,
};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 小米电脑管家相关进程（不含扩展名），用于启动时全量关闭的兜底匹配。
pub const PROC_MIPCM_ALL: &[&str] = &[
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
pub const PROC_LOCALE: &[&str] = &["micont_service"];
pub const PROC_CAMERA: &[&str] = &["XiaomiPcManager"];
pub const PROC_AUDIO: &[&str] = &["MiPCAudio", "MiPlayCastService", "MAFSvr", "MASFvr"];
pub const PROC_DEVICE: &[&str] = &["XiaomiPcManager"];
pub const PROC_SMBIOS: &[&str] = &["micont_service"];

const RESTART_HINT: &str = "提示：补丁已完成，请手动重新启动小米电脑管家使其生效。";

/// 音频广播介质。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BroadcastMode {
    Wireless,
    Wired,
}

impl From<BroadcastMode> for mipcaudio_lan::BroadcastMode {
    fn from(mode: BroadcastMode) -> Self {
        match mode {
            BroadcastMode::Wireless => mipcaudio_lan::BroadcastMode::Wireless,
            BroadcastMode::Wired => mipcaudio_lan::BroadcastMode::Wired,
        }
    }
}

// ===================== 探测 / 可用性 =====================

/// 是否探测到完整版小米电脑管家（决定摄像头/音频/设备伪装是否可用）。
pub fn full_features_available() -> bool {
    install::find_install_root().is_some()
}

/// 启动时关闭所有小米电脑管家相关进程，返回提示（无进程被关闭时返回 None）。
pub fn close_all_on_startup() -> Option<String> {
    let n = install::kill_mipcmanager_processes(PROC_MIPCM_ALL);
    (n > 0).then(|| format!("已关闭 {n} 个小米电脑管家相关进程。"))
}

// ===================== 状态汇总 =====================

/// 生成与 CLI `status` 一致的状态文本行。
pub fn status_lines() -> Vec<String> {
    let mut out = Vec::new();
    out.push("== 小米电脑管家补丁状态 ==".to_string());
    let manager_root = install::find_install_root();
    let continuity_root = install::find_pc_continuity_root();
    if manager_root.is_none() && continuity_root.is_none() {
        out.push("未探测到安装目录（可用 --dll/--dir 手动指定）。".to_string());
        return out;
    }
    if let Some(root) = manager_root {
        out.push(String::new());
        out.push("-- XiaomiPCManager（全功能）--".to_string());
        push_full_installation_status(&root, &mut out);
    }
    if let Some(root) = continuity_root {
        out.push(String::new());
        out.push("-- PcContinuity（小米互联 / 互联互通，仅地区伪装）--".to_string());
        out.push(format!("安装根目录：{}", root.display()));
        match install::latest_version_dir(&root) {
            Ok(version) => {
                out.push(format!("最新版本目录：{}", version.display()));
                push_file_status(&version.join(locale_spoof::TARGET_DLL), &mut out);
                out.push("  摄像头、音频流转和设备伪装：当前版本不可用".to_string());
            }
            Err(error) => out.push(format!("（无法确定版本目录：{error}）")),
        }
    }
    out
}

fn push_full_installation_status(root: &Path, out: &mut Vec<String>) {
    out.push(format!("安装根目录：{}", root.display()));
    match install::latest_version_dir(root) {
        Ok(version) => {
            out.push(format!("最新版本目录：{}", version.display()));
            push_file_status(&version.join(locale_spoof::TARGET_DLL), out);
            push_file_status(&version.join(camera_toast::TARGET_DLL), out);
            out.push("  -- 音频流转广播模式 --".to_string());
            for (file, state) in mipcaudio_lan::current_state(&version) {
                out.push(format!("     {file}: {state}"));
            }
            out.push(format!(
                "     Wi-Fi 本地路由: {}",
                audio_wifi_route::state(&version)
            ));
            out.push("  -- 设备伪装 --".to_string());
            let (dll_ok, model) = device_spoof::current_state(&version);
            out.push(format!(
                "     msimg32.dll: {} | 注册表机型: {}",
                if dll_ok { "已就位" } else { "未就位" },
                model.unwrap_or_else(|| "未设置".to_string())
            ));
            out.push("  -- [实验性] Lyra 特殊适配 --".to_string());
            let smbios_dll = version.join(smbios_spoof::TARGET_DLL);
            let smbios_ok = smbios_dll.exists() && smbios_spoof::is_patched(&smbios_dll);
            out.push(format!(
                "     SMBIOS 设备身份: {}",
                if smbios_ok { "已就位" } else { "未就位" }
            ));
        }
        Err(error) => out.push(format!("（无法确定版本目录：{error}）")),
    }
}

fn push_file_status(path: &Path, out: &mut Vec<String>) {
    let exists = path.exists();
    let bak = install::backup_path(path).exists();
    out.push(format!(
        "  {} | 存在:{} | 备份:{}",
        path.file_name().unwrap().to_string_lossy(),
        if exists { "是" } else { "否" },
        if bak { "有" } else { "无" }
    ));
}

// ===================== 地区伪装 =====================

pub fn apply_locale(
    dll: Option<PathBuf>,
    region: &str,
    write_registry: bool,
    no_kill: bool,
) -> Result<Vec<String>> {
    let path = resolve_locale_dll(dll)?;
    let mut log = Vec::new();
    close_apps_required(PROC_LOCALE, no_kill, &mut log)?;
    let outcome = retry_patch_after_access_denied(PROC_LOCALE, &mut log, || {
        locale_spoof::apply(&path, region, write_registry)
    })?;
    match outcome {
        locale_spoof::PatchOutcome::Patched => {
            log.push(format!("✓ 地区伪装已应用：{}", path.display()));
        }
        locale_spoof::PatchOutcome::AlreadyPatched => {
            log.push(format!("• DLL 已是补丁状态（跳过）：{}", path.display()));
        }
    }
    if write_registry {
        log.push(format!(
            "  注册表 HKCU\\Control Panel\\International\\Geo\\XCN = {region}"
        ));
    }
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

pub fn revert_locale(
    dll: Option<PathBuf>,
    remove_registry: bool,
    no_kill: bool,
) -> Result<Vec<String>> {
    let path = resolve_locale_dll(dll)?;
    let mut log = Vec::new();
    close_apps_required(PROC_LOCALE, no_kill, &mut log)?;
    retry_patch_after_access_denied(PROC_LOCALE, &mut log, || {
        locale_spoof::revert(&path, remove_registry)
    })?;
    log.push(format!("✓ 已还原地区伪装：{}", path.display()));
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

// ===================== 摄像头弹窗 =====================

pub fn apply_camera(dll: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let path = resolve_full_feature_dll(dll, camera_toast::TARGET_DLL)?;
    let mut log = Vec::new();
    close_apps(PROC_CAMERA, no_kill, &mut log);
    let outcome =
        retry_patch_after_access_denied(PROC_CAMERA, &mut log, || camera_toast::apply(&path))?;
    match outcome {
        dotnet::InjectOutcome::Patched => {
            log.push(format!("✓ 摄像头弹窗补丁已应用：{}", path.display()));
        }
        dotnet::InjectOutcome::AlreadyPatched => {
            log.push(format!("• 已是补丁状态（跳过）：{}", path.display()));
        }
    }
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

pub fn revert_camera(dll: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let path = resolve_full_feature_dll(dll, camera_toast::TARGET_DLL)?;
    let mut log = Vec::new();
    close_apps(PROC_CAMERA, no_kill, &mut log);
    retry_patch_after_access_denied(PROC_CAMERA, &mut log, || camera_toast::revert(&path))?;
    log.push(format!("✓ 已还原摄像头弹窗补丁：{}", path.display()));
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

// ===================== 音频流转 =====================

/// 应用音频补丁；GUI 等不需要公开高级选项的调用方使用默认的双网卡修复行为。
pub fn apply_audio(
    mode: BroadcastMode,
    dir: Option<PathBuf>,
    no_kill: bool,
) -> Result<Vec<String>> {
    apply_audio_with_options(mode, dir, no_kill, false)
}

/// 应用音频补丁，并允许 CLI 显式关闭 Wi-Fi 本地子网路由修复。
///
/// 路由修复仅在 WiFi 模式下需要：当有线 + Wi-Fi 位于同一子网时，有线因跃点更低会抢走
/// 音频媒体会话的出站流量，导致会话来源 IP 与发现身份不一致，手机立即 TEARDOWN。
/// 在 Wi-Fi 子网上添加 metric=1 的持久路由可强制媒体会话走 Wi-Fi。
///
/// 有线模式下无需路由修复：发现身份已切换为有线 MAC，默认路由自然走有线（有线跃点更低），
/// 两者一致。
pub fn apply_audio_with_options(
    mode: BroadcastMode,
    dir: Option<PathBuf>,
    no_kill: bool,
    no_wifi_local_route: bool,
) -> Result<Vec<String>> {
    let dir = resolve_full_version_dir_or(dir)?;
    let mut log = Vec::new();
    close_apps(PROC_AUDIO, no_kill, &mut log);
    let patch_mode: mipcaudio_lan::BroadcastMode = mode.into();
    let results = retry_patch_after_access_denied(PROC_AUDIO, &mut log, || {
        mipcaudio_lan::apply(&dir, patch_mode)
    })?;
    log.push(format!(
        "✓ 音频流转广播模式已切换为：{}",
        patch_mode.label()
    ));
    for (file, outcome) in results {
        log.push(format!(
            "  {file}: 改写 {} 处, 已是目标 {} 处",
            outcome.patched, outcome.already
        ));
    }
    log.push("  （三处网卡身份已统一 → 手机端应为单设备）".to_string());
    match (mode, no_wifi_local_route) {
        (BroadcastMode::Wireless, false) => match audio_wifi_route::apply(&dir)? {
            Some(true) => log.push(
                "  已添加 Wi-Fi 本地子网优先路由：音频会话走 Wi-Fi，本机默认流量仍走有线。"
                    .to_string(),
            ),
            Some(false) => log.push("  Wi-Fi 本地子网优先路由已存在。".to_string()),
            None => log.push(
                "  未检测到可用 Wi-Fi IPv4 接口；已保留无线广播补丁，未添加本地路由。".to_string(),
            ),
        },
        (BroadcastMode::Wired, _) => {
            if audio_wifi_route::revert(&dir)? {
                log.push(
                    "  已移除 Wi-Fi 本地子网优先路由（有线模式下发现与媒体均走有线，无需此路由）。"
                        .to_string(),
                );
            }
        }
        (BroadcastMode::Wireless, true) => {}
    }
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

pub fn revert_audio(dir: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let dir = resolve_full_version_dir_or(dir)?;
    let mut log = Vec::new();
    close_apps(PROC_AUDIO, no_kill, &mut log);
    retry_patch_after_access_denied(PROC_AUDIO, &mut log, || mipcaudio_lan::revert(&dir))?;
    if audio_wifi_route::revert(&dir)? {
        log.push("  已移除 Wi-Fi 本地子网优先路由。".to_string());
    }
    log.push("✓ 已还原音频流转补丁（MiPCAudio.exe / idmruntime.dll）".to_string());
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

// ===================== 设备伪装 =====================

pub fn apply_device(model: &str, dir: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let dir = resolve_full_version_dir_or(dir)?;
    let mut log = Vec::new();
    close_apps(PROC_DEVICE, no_kill, &mut log);
    retry_patch_after_access_denied(PROC_DEVICE, &mut log, || device_spoof::apply(&dir, model))?;
    log.push(format!("✓ 设备伪装已应用：机型 = {model}"));
    log.push(format!(
        "  已释放 {} 至 {}",
        device_spoof::PROXY_DLL_NAME,
        dir.display()
    ));
    log.push(format!(
        "  注册表 HKCU\\Software\\SmartSharePatch\\SpoofDevice = {model}"
    ));
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

pub fn revert_device(dir: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let dir = resolve_full_version_dir_or(dir)?;
    let mut log = Vec::new();
    close_apps(PROC_DEVICE, no_kill, &mut log);
    retry_patch_after_access_denied(PROC_DEVICE, &mut log, || device_spoof::revert(&dir))?;
    log.push("✓ 已还原设备伪装（移除 msimg32.dll 与注册表机型）".to_string());
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

// ===================== SMBIOS 伪装 =====================

pub fn apply_smbios(
    model: Option<&str>,
    dll: Option<PathBuf>,
    no_kill: bool,
) -> Result<Vec<String>> {
    let path = resolve_full_feature_dll(dll, smbios_spoof::TARGET_DLL)?;
    let mut log = Vec::new();
    close_apps_required(PROC_SMBIOS, no_kill, &mut log)?;
    let outcome = retry_patch_after_access_denied(PROC_SMBIOS, &mut log, || {
        smbios_spoof::apply(&path, model)
    })?;
    match outcome {
        smbios_spoof::PatchOutcome::Patched => {
            log.push(format!("✓ SMBIOS 设备身份补丁已应用：{}", path.display()));
        }
        smbios_spoof::PatchOutcome::AlreadyPatched => {
            log.push(format!("• SMBIOS 已是补丁状态（跳过）：{}", path.display()));
        }
    }
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

pub fn revert_smbios(dll: Option<PathBuf>, no_kill: bool) -> Result<Vec<String>> {
    let path = resolve_full_feature_dll(dll, smbios_spoof::TARGET_DLL)?;
    let mut log = Vec::new();
    close_apps_required(PROC_SMBIOS, no_kill, &mut log)?;
    retry_patch_after_access_denied(PROC_SMBIOS, &mut log, || smbios_spoof::revert(&path))?;
    log.push(format!("✓ 已还原 SMBIOS 设备身份补丁：{}", path.display()));
    log.push(RESTART_HINT.to_string());
    Ok(log)
}

// ===================== 安装 =====================

/// 根据所选安装包安装小米电脑管家 / 小米互联：自动识别产品、校验共存、启动安装。
pub fn install_from_path(installer: &Path) -> Result<Vec<String>> {
    let kind = pc_manager_installer::classify_installer(installer);
    let manager_root = install::find_install_root();
    let continuity_root = install::find_pc_continuity_root();
    ensure_install_allowed(kind, manager_root.as_deref(), continuity_root.as_deref())?;
    let pid = pc_manager_installer::launch_installer(installer)?;
    Ok(vec![
        format!("✓ 已启动{}安装包：{}", kind.label(), installer.display()),
        format!(
            "  已释放 {}、写入 SpoofDevice={}，注入并旁路 WinVersionMatch(需Win11) + MatchProduct*| PID: {pid}",
            device_spoof::PROXY_DLL_NAME,
            device_spoof::DEFAULT_MODEL
        ),
    ])
}

/// 校验所选安装包所属产品能否安装：两个产品不允许同时安装。
pub fn ensure_install_allowed(
    kind: pc_manager_installer::InstallerKind,
    manager_root: Option<&Path>,
    continuity_root: Option<&Path>,
) -> Result<()> {
    use pc_manager_installer::InstallerKind;
    match kind {
        InstallerKind::XiaomiPcManager => {
            if let Some(root) = continuity_root {
                bail!(
                    "已安装小米互联 / 互联互通（{}），官方不支持与小米电脑管家同时安装",
                    root.display()
                );
            }
        }
        InstallerKind::PcContinuity => {
            if let Some(root) = manager_root {
                bail!(
                    "已安装小米电脑管家（{}），不支持与小米互联 / 互联互通同时安装",
                    root.display()
                );
            }
        }
    }
    Ok(())
}

// ===================== 目标路径解析 =====================

/// 解析地区伪装 DLL：优先显式路径，否则先找完整版电脑管家，再找 PcContinuity。
pub fn resolve_locale_dll(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.exists() {
            bail!("指定的文件不存在：{}", p.display());
        }
        return Ok(p);
    }
    let manager_root = install::find_install_root();
    let continuity_root = install::find_pc_continuity_root();
    resolve_locale_dll_from_roots(manager_root.as_deref(), continuity_root.as_deref())
}

pub fn resolve_locale_dll_from_roots(
    manager_root: Option<&Path>,
    continuity_root: Option<&Path>,
) -> Result<PathBuf> {
    let mut errors = Vec::new();
    for root in [manager_root, continuity_root].into_iter().flatten() {
        match install::latest_version_dir(root) {
            Ok(version) => {
                let dll = version.join(locale_spoof::TARGET_DLL);
                if dll.exists() {
                    return Ok(dll);
                }
                errors.push(format!(
                    "在 {} 中未找到 {}",
                    version.display(),
                    locale_spoof::TARGET_DLL
                ));
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    if errors.is_empty() {
        bail!("未找到 XiaomiPCManager 或 PcContinuity 安装目录");
    }
    bail!(
        "未找到可用的 {}：{}",
        locale_spoof::TARGET_DLL,
        errors.join("；")
    )
}

/// 解析仅完整版电脑管家支持的 DLL。
pub fn resolve_full_feature_dll(explicit: Option<PathBuf>, filename: &str) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if !path.exists() {
            bail!("指定的文件不存在：{}", path.display());
        }
        let continuity_root = install::find_pc_continuity_root();
        ensure_full_feature_path_supported(&path, continuity_root.as_deref())?;
        return Ok(path);
    }
    let version = resolve_full_version_dir()?;
    let path = version.join(filename);
    if !path.exists() {
        bail!("在 {} 中未找到 {filename}", version.display());
    }
    Ok(path)
}

/// 探测完整版电脑管家的最新版本目录。
pub fn resolve_full_version_dir() -> Result<PathBuf> {
    let manager_root = install::find_install_root();
    let continuity_root = install::find_pc_continuity_root();
    resolve_full_version_dir_from_roots(manager_root.as_deref(), continuity_root.as_deref())
}

pub fn resolve_full_version_dir_from_roots(
    manager_root: Option<&Path>,
    continuity_root: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(root) = manager_root {
        return install::latest_version_dir(root);
    }
    if continuity_root.is_some() {
        bail!("PcContinuity 暂时仅支持地区伪装，其他功能不可用");
    }
    bail!("未找到 XiaomiPCManager 安装目录")
}

/// 显式版本目录优先，否则自动探测完整版电脑管家。
pub fn resolve_full_version_dir_or(explicit: Option<PathBuf>) -> Result<PathBuf> {
    match explicit {
        Some(d) if d.is_dir() => {
            let continuity_root = install::find_pc_continuity_root();
            ensure_full_feature_path_supported(&d, continuity_root.as_deref())?;
            Ok(d)
        }
        Some(d) => bail!("指定的版本目录不存在：{}", d.display()),
        None => resolve_full_version_dir(),
    }
}

pub fn ensure_full_feature_path_supported(
    path: &Path,
    continuity_root: Option<&Path>,
) -> Result<()> {
    let Some(root) = continuity_root else {
        return Ok(());
    };
    let normalized_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if normalized_path.starts_with(&normalized_root) {
        bail!("PcContinuity 暂时仅支持地区伪装，其他功能不可用");
    }
    Ok(())
}

// ===================== 进程关闭 / 重试 =====================

/// 打补丁前关闭相关进程（软失败：结束不了也继续）。
fn close_apps(procs: &[&str], no_kill: bool, log: &mut Vec<String>) {
    if no_kill {
        return;
    }
    let n = install::kill_by_names(procs);
    if n > 0 {
        log.push(format!("已关闭 {n} 个相关进程：{}", procs.join(", ")));
    }
}

/// 需要确保进程已退出的补丁操作使用该函数；若仍在运行则拒绝继续 Patch。
fn close_apps_required(procs: &[&str], no_kill: bool, log: &mut Vec<String>) -> Result<()> {
    let (n, running) = if no_kill {
        (0, install::running_by_names(procs))
    } else {
        install::kill_by_names_until_gone(procs, Duration::from_secs(5))
    };
    if n > 0 {
        log.push(format!("已关闭 {n} 个相关进程：{}", procs.join(", ")));
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
fn retry_patch_after_access_denied<T, F>(
    procs: &[&str],
    log: &mut Vec<String>,
    mut patch: F,
) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    match patch() {
        Ok(v) => Ok(v),
        Err(e) if is_access_denied(&e) => {
            log.push(format!(
                "遇到拒绝访问，正在关闭相关进程后重试一次：{}",
                procs.join(", ")
            ));
            let (n, running) = install::kill_by_names_until_gone(procs, Duration::from_secs(5));
            if n > 0 {
                log.push(format!("已关闭 {n} 个相关进程：{}", procs.join(", ")));
            }
            if !running.is_empty() {
                log.push(format!(
                    "以下进程仍在运行，将按要求重试一次：{}",
                    running.join(", ")
                ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mipcm_ops_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn locale_auto_resolution_uses_pc_continuity() {
        let continuity_root = fixture_root("locale_continuity");
        let version = continuity_root.join("1.1.2.36");
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join(locale_spoof::TARGET_DLL), b"fixture").unwrap();

        let resolved = resolve_locale_dll_from_roots(None, Some(&continuity_root)).unwrap();

        assert_eq!(resolved, version.join(locale_spoof::TARGET_DLL));
        fs::remove_dir_all(continuity_root).unwrap();
    }

    #[test]
    fn full_features_are_unavailable_for_pc_continuity_only() {
        let continuity_root = fixture_root("full_feature_continuity");
        fs::create_dir_all(continuity_root.join("1.1.2.36")).unwrap();

        let error = resolve_full_version_dir_from_roots(None, Some(&continuity_root))
            .unwrap_err()
            .to_string();

        assert!(error.contains("PcContinuity 暂时仅支持地区伪装"));
        fs::remove_dir_all(continuity_root).unwrap();
    }

    #[test]
    fn explicit_full_feature_path_inside_pc_continuity_is_rejected() {
        let continuity_root = fixture_root("explicit_continuity");
        let version = continuity_root.join("1.1.2.36");
        fs::create_dir_all(&version).unwrap();

        let error = ensure_full_feature_path_supported(&version, Some(&continuity_root))
            .unwrap_err()
            .to_string();

        assert!(error.contains("PcContinuity 暂时仅支持地区伪装"));
        fs::remove_dir_all(continuity_root).unwrap();
    }

    #[test]
    fn install_gating_rejects_coexisting_products() {
        use pc_manager_installer::InstallerKind;
        let manager_root = Path::new(r"C:\Program Files\MI\XiaomiPCManager");
        let continuity_root = Path::new(r"C:\Program Files\MI\PcContinuity");

        // 全新环境：两种产品都可安装。
        assert!(ensure_install_allowed(InstallerKind::XiaomiPcManager, None, None).is_ok());
        assert!(ensure_install_allowed(InstallerKind::PcContinuity, None, None).is_ok());

        // 已装小米互联：可继续安装/升级小米互联，但不允许再装小米电脑管家。
        assert!(
            ensure_install_allowed(InstallerKind::PcContinuity, None, Some(continuity_root))
                .is_ok()
        );
        let error =
            ensure_install_allowed(InstallerKind::XiaomiPcManager, None, Some(continuity_root))
                .unwrap_err()
                .to_string();
        assert!(error.contains("已安装小米互联"));

        // 已装小米电脑管家：可继续安装/升级，但不允许再装小米互联。
        assert!(
            ensure_install_allowed(InstallerKind::XiaomiPcManager, Some(manager_root), None)
                .is_ok()
        );
        let error = ensure_install_allowed(InstallerKind::PcContinuity, Some(manager_root), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("已安装小米电脑管家"));
    }
}
