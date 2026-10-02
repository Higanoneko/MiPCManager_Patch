//! Windows 11 一级右键菜单中的“小米互传”集成。
//!
//! Shell Extension 基于 cnbluefire/MiDropShellExtForWindows11（MIT）并由
//! YYplus/XiaomiShareShellExt-Minimal 精简、验证。二进制载荷在编译期内嵌，
//! 启用时释放到当前用户 LocalAppData，并通过 sparse MSIX 注册 IExplorerCommand。

use crate::infra::powershell::{run_powershell_strict, run_powershell_with_env_strict};
use anyhow::{Context, Result, anyhow, bail};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

const PACKAGE_NAME: &str = "XiaomiShareShellExt.Minimal";
const PACKAGE_VERSION: &str = "1.0.2.0";
const UPSTREAM_PACKAGE_NAME: &str = "5f71dad9-3e77-4ada-9fad-12c2e761288f";
const CERT_THUMBPRINT: &str = "2116B926D060DD8A9FA1086F9A67FD8F0EE9D291";

// 只删除本项目已知构建所使用的证书。绝不使用 LocalAppData 中的指纹文件决定
// 要删除哪个证书，避免用户可写文件扩大管理员权限下的删除范围。
const MANAGED_CERT_THUMBPRINTS: &[&str] = &[
    CERT_THUMBPRINT,
    "642CFE5305308D77F09D78F16BA66C067280BFD0", // standalone v1.0.1
    "942641EDC8B0DDDA817FFFF37C6F301F00EEA36D", // standalone v1.0.0
];

const IDENTITY_MSIX: &[u8] = include_bytes!("bin/XiaomiShareShellExt.Identity.msix");
const CERTIFICATE: &[u8] = include_bytes!("bin/XiaomiShareShellExt.cer");
const RUNTIME_FILES: &[(&str, &[u8])] = &[
    (
        "XiaomiShare.ShellExt/XiaomiShare.ShellExt.dll",
        include_bytes!("bin/XiaomiShare.ShellExt.dll"),
    ),
    (
        "XiaomiShare.Helper/XiaomiShare.Helper.exe",
        include_bytes!("bin/XiaomiShare.Helper.exe"),
    ),
    (
        "Assets/Square44x44Logo.png",
        include_bytes!("bin/Square44x44Logo.png"),
    ),
    (
        "Assets/Square150x150Logo.png",
        include_bytes!("bin/Square150x150Logo.png"),
    ),
    ("Assets/StoreLogo.png", include_bytes!("bin/StoreLogo.png")),
    ("THIRD-PARTY-NOTICES.txt", include_bytes!("NOTICE.md")),
];

static OPERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMenuState {
    Disabled,
    Enabled,
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchOutcome {
    Applied,
    AlreadyApplied,
    Reverted,
    AlreadyReverted,
}

fn lock_operation() -> Result<MutexGuard<'static, ()>> {
    OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow!("右键菜单操作锁已损坏，请重新启动程序后重试"))
}

fn install_root() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").context("无法获取 LOCALAPPDATA")?;
    Ok(PathBuf::from(local)
        .join("Programs")
        .join("XiaomiShareShellExt"))
}

fn cache_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("XiaomiShareShellExt"))
}

fn staging_root(root: &Path) -> PathBuf {
    root.with_file_name("XiaomiShareShellExt.staging")
}

fn file_matches(path: &Path, expected: &[u8]) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes == expected)
}

fn runtime_payload_complete(root: &Path) -> bool {
    RUNTIME_FILES
        .iter()
        .all(|(name, bytes)| file_matches(&root.join(name), bytes))
        && fs::read_to_string(root.join("certificate-thumbprint.txt"))
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case(CERT_THUMBPRINT))
}

/// 返回当前右键菜单集成状态。状态查询失败时保守地报告为不完整。
pub fn current_state() -> ShellMenuState {
    match package_version(PACKAGE_NAME) {
        Ok(package) => state_from_package(package),
        Err(_) => ShellMenuState::Partial,
    }
}

fn current_state_checked() -> Result<ShellMenuState> {
    Ok(state_from_package(package_version(PACKAGE_NAME)?))
}

fn state_from_package(package: Option<String>) -> ShellMenuState {
    let Ok(root) = install_root() else {
        return ShellMenuState::Partial;
    };
    let payload_complete = runtime_payload_complete(&root);

    match package {
        None if !root.exists() => ShellMenuState::Disabled,
        Some(version) if payload_complete && version.eq_ignore_ascii_case(PACKAGE_VERSION) => {
            ShellMenuState::Enabled
        }
        _ => ShellMenuState::Partial,
    }
}

/// 启用 Windows 11 一级右键“小米互传”入口。重复执行幂等。
pub fn apply() -> Result<PatchOutcome> {
    let _guard = lock_operation()?;
    ensure_supported_windows()?;
    if package_version(UPSTREAM_PACKAGE_NAME)?.is_some() {
        bail!("检测到原版 MiDropShellExtForWindows11，请先卸载该扩展以避免重复右键菜单项");
    }

    let installed_package = package_version(PACKAGE_NAME)?;
    if state_from_package(installed_package.clone()) == ShellMenuState::Enabled {
        return Ok(PatchOutcome::AlreadyApplied);
    }

    let root = install_root()?;
    let staging = staging_root(&root);
    if let Err(error) = prepare_payload(&staging) {
        let cleanup = remove_runtime_dir_with_retry(&staging);
        return match cleanup {
            Ok(()) => Err(error.context("准备右键菜单载荷失败")),
            Err(cleanup_error) => Err(error.context(format!(
                "准备右键菜单载荷失败；清理 staging 也失败：{cleanup_error:#}"
            ))),
        };
    }

    let had_package = installed_package.is_some();
    let had_runtime = root.exists();

    // 若旧 package 连注销都失败，保留旧安装，只清理尚未启用的 staging。
    if had_package && let Err(error) = remove_package(PACKAGE_NAME) {
        let cleanup = remove_runtime_dir_with_retry(&staging);
        return match cleanup {
            Ok(()) => Err(error.context("无法注销旧版右键菜单 package")),
            Err(cleanup_error) => Err(error.context(format!(
                "无法注销旧版右键菜单 package；清理 staging 也失败：{cleanup_error:#}"
            ))),
        };
    }

    let result = (|| -> Result<()> {
        if had_package || had_runtime {
            restart_explorer()?;
            thread::sleep(Duration::from_millis(500));
        }

        remove_managed_certificates()?;
        remove_runtime_dir_with_retry(&root)?;
        fs::rename(&staging, &root).with_context(|| {
            format!(
                "无法启用已准备的右键菜单载荷 {} -> {}",
                staging.display(),
                root.display()
            )
        })?;

        let msix_path = root.join("XiaomiShareShellExt.Identity.msix");
        let cert_path = root.join("XiaomiShareShellExt.cer");
        import_certificate(&cert_path)?;
        add_package(&msix_path, &root)?;

        remove_file_if_exists(&msix_path)?;
        remove_file_if_exists(&cert_path)?;

        restart_explorer()?;
        thread::sleep(Duration::from_millis(500));
        if current_state_checked()? != ShellMenuState::Enabled {
            bail!("右键菜单扩展注册后状态校验失败");
        }
        Ok(())
    })();

    if let Err(error) = result {
        let cleanup = cleanup_failed_apply(&root, &staging);
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup_error) => {
                Err(error.context(format!("失败后的清理也失败：{cleanup_error:#}")))
            }
        };
    }

    Ok(PatchOutcome::Applied)
}

/// 禁用右键菜单并清理 package、证书、缓存与运行载荷。重复执行幂等。
pub fn revert() -> Result<PatchOutcome> {
    let _guard = lock_operation()?;
    let root = install_root()?;
    let staging = staging_root(&root);
    let package_present = package_version(PACKAGE_NAME)?.is_some();
    let root_present = root.exists();
    let staging_present = staging.exists();
    let cache = cache_root();
    let cache_present = cache.as_ref().is_some_and(|path| path.exists());
    let certificate_present = managed_certificates_present()?;
    let had_managed_state =
        package_present || root_present || staging_present || cache_present || certificate_present;

    if package_present {
        remove_package(PACKAGE_NAME)?;
    }
    if package_present || root_present {
        restart_explorer()?;
        thread::sleep(Duration::from_millis(500));
    }

    remove_managed_certificates()?;
    if let Some(cache) = cache.as_deref() {
        remove_dir_if_exists(cache)
            .with_context(|| format!("无法清理右键菜单缓存 {}", cache.display()))?;
    }
    remove_runtime_dir_with_retry(&root)?;
    remove_runtime_dir_with_retry(&staging)?;

    if package_version(PACKAGE_NAME)?.is_some() {
        bail!("右键菜单扩展卸载后 package 仍然存在");
    }
    if managed_certificates_present()? {
        bail!("右键菜单扩展卸载后受管理证书仍然存在");
    }
    if root.exists() || staging.exists() || cache.as_ref().is_some_and(|path| path.exists()) {
        bail!("右键菜单扩展卸载后仍有受管理文件残留");
    }

    Ok(if had_managed_state {
        PatchOutcome::Reverted
    } else {
        PatchOutcome::AlreadyReverted
    })
}

fn prepare_payload(root: &Path) -> Result<()> {
    remove_runtime_dir_with_retry(root)?;
    for name in ["XiaomiShare.ShellExt", "XiaomiShare.Helper", "Assets"] {
        let dir = root.join(name);
        fs::create_dir_all(&dir).with_context(|| format!("无法创建 {}", dir.display()))?;
    }
    for (name, bytes) in RUNTIME_FILES {
        write_embedded_file(&root.join(name), bytes)?;
    }
    write_embedded_file(
        &root.join("XiaomiShareShellExt.Identity.msix"),
        IDENTITY_MSIX,
    )?;
    write_embedded_file(&root.join("XiaomiShareShellExt.cer"), CERTIFICATE)?;
    fs::write(root.join("certificate-thumbprint.txt"), CERT_THUMBPRINT)
        .context("无法写入证书指纹")?;

    if !runtime_payload_complete(root)
        || !file_matches(
            &root.join("XiaomiShareShellExt.Identity.msix"),
            IDENTITY_MSIX,
        )
        || !file_matches(&root.join("XiaomiShareShellExt.cer"), CERTIFICATE)
    {
        bail!("右键菜单内嵌载荷写入后的完整性校验失败");
    }
    Ok(())
}

fn write_embedded_file(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).with_context(|| format!("无法写入 {}", path.display()))
}

fn cleanup_failed_apply(root: &Path, staging: &Path) -> Result<()> {
    // package 仍然注册时绝不能删除其外部载荷或信任证书；否则会把一个可恢复的
    // 注册失败扩大成一个指向已删除文件的 package。只清理尚未启用的 staging。
    let package_cleanup = remove_package(PACKAGE_NAME).and_then(|()| {
        if package_version(PACKAGE_NAME)?.is_some() {
            bail!("注销命令返回成功，但 package 仍然存在");
        }
        Ok(())
    });
    if let Err(error) = package_cleanup {
        return match remove_runtime_dir_with_retry(staging) {
            Ok(()) => Err(error.context("无法安全注销失败安装的 package；已保留其运行载荷和证书")),
            Err(staging_error) => Err(error.context(format!(
                "无法安全注销失败安装的 package；已保留其运行载荷和证书；清理 staging 也失败：{staging_error:#}"
            ))),
        };
    }

    let mut errors = Vec::new();
    collect_cleanup_error(&mut errors, "刷新 Explorer", restart_explorer());
    thread::sleep(Duration::from_millis(500));
    collect_cleanup_error(&mut errors, "删除证书", remove_managed_certificates());
    if let Some(cache) = cache_root() {
        collect_cleanup_error(&mut errors, "删除缓存", remove_dir_if_exists(&cache));
    }
    collect_cleanup_error(
        &mut errors,
        "删除运行载荷",
        remove_runtime_dir_with_retry(root),
    );
    collect_cleanup_error(
        &mut errors,
        "删除 staging",
        remove_runtime_dir_with_retry(staging),
    );

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("{}", errors.join("；"))
    }
}

fn collect_cleanup_error(errors: &mut Vec<String>, label: &str, result: Result<()>) {
    if let Err(error) = result {
        errors.push(format!("{label}失败：{error:#}"));
    }
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法删除 {}", path.display())),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法删除目录 {}", path.display())),
    }
}

fn remove_runtime_dir_with_retry(root: &Path) -> Result<()> {
    let mut last_error = None;
    for _ in 0..10 {
        match fs::remove_dir_all(root) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
    let error = last_error.context("删除右键菜单运行目录失败")?;
    Err(error).with_context(|| format!("无法删除运行目录 {}", root.display()))
}

fn package_version(name: &str) -> Result<Option<String>> {
    let output = run_powershell_with_env_strict(
        "$p = Get-AppxPackage -Name $env:MIPCM_PACKAGE_NAME -ErrorAction Stop | Select-Object -First 1; if ($p) { $p.Version.ToString() }",
        &[("MIPCM_PACKAGE_NAME", OsStr::new(name))],
    )?;
    let version = output.trim();
    Ok((!version.is_empty()).then(|| version.to_string()))
}

fn add_package(msix: &Path, external_location: &Path) -> Result<()> {
    run_powershell_with_env_strict(
        "Add-AppxPackage -Path $env:MIPCM_MSIX_PATH -ExternalLocation $env:MIPCM_EXTERNAL_LOCATION -ErrorAction Stop | Out-Null",
        &[
            ("MIPCM_MSIX_PATH", msix.as_os_str()),
            ("MIPCM_EXTERNAL_LOCATION", external_location.as_os_str()),
        ],
    )?;
    Ok(())
}

fn remove_package(name: &str) -> Result<()> {
    run_powershell_with_env_strict(
        "Get-AppxPackage -Name $env:MIPCM_PACKAGE_NAME -ErrorAction Stop | ForEach-Object { Remove-AppxPackage -Package $_.PackageFullName -ErrorAction Stop }",
        &[("MIPCM_PACKAGE_NAME", OsStr::new(name))],
    )?;
    Ok(())
}

fn import_certificate(cert: &Path) -> Result<()> {
    run_powershell_with_env_strict(
        "Import-Certificate -FilePath $env:MIPCM_CERT_PATH -CertStoreLocation 'Cert:\\LocalMachine\\TrustedPeople' -ErrorAction Stop | Out-Null",
        &[("MIPCM_CERT_PATH", cert.as_os_str())],
    )?;
    Ok(())
}

fn managed_certificate_paths() -> String {
    let mut paths = Vec::with_capacity(MANAGED_CERT_THUMBPRINTS.len() * 2);
    for thumbprint in MANAGED_CERT_THUMBPRINTS {
        paths.push(format!("Cert:\\LocalMachine\\TrustedPeople\\{thumbprint}"));
        paths.push(format!("Cert:\\CurrentUser\\TrustedPeople\\{thumbprint}"));
    }
    paths.join("\n")
}

fn remove_managed_certificates() -> Result<()> {
    let paths = managed_certificate_paths();
    run_powershell_with_env_strict(
        "$env:MIPCM_CERT_PATHS -split \"`n\" | Where-Object { $_ } | ForEach-Object { if (Test-Path -LiteralPath $_ -ErrorAction Stop) { Remove-Item -LiteralPath $_ -Force -ErrorAction Stop } }",
        &[("MIPCM_CERT_PATHS", OsStr::new(&paths))],
    )?;
    Ok(())
}

fn managed_certificates_present() -> Result<bool> {
    let paths = managed_certificate_paths();
    let output = run_powershell_with_env_strict(
        "$found = $false; $env:MIPCM_CERT_PATHS -split \"`n\" | Where-Object { $_ } | ForEach-Object { if (Test-Path -LiteralPath $_ -ErrorAction Stop) { $found = $true } }; if ($found) { '1' } else { '0' }",
        &[("MIPCM_CERT_PATHS", OsStr::new(&paths))],
    )?;
    match output.trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        other => bail!("证书状态查询返回异常结果：{other:?}"),
    }
}

fn restart_explorer() -> Result<()> {
    run_powershell_strict(
        "$session = (Get-Process -Id $PID -ErrorAction Stop).SessionId; Get-Process -Name explorer -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session } | Stop-Process -Force -ErrorAction Stop",
    )?;
    Ok(())
}

fn ensure_supported_windows() -> Result<()> {
    let build = run_powershell_strict("[Environment]::OSVersion.Version.Build")?;
    let build = build.trim().parse::<u32>().unwrap_or(0);
    if build < 22000 {
        bail!("Windows 11 一级右键菜单扩展需要 Windows 11（build 22000 或更高版本）");
    }
    Ok(())
}
