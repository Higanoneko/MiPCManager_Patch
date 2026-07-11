use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 在 Patcher 同目录中查找 `*_XiaomiPCManager_*.exe` 安装包。
pub fn find_local_installers(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut installers = Vec::new();
    let current_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    for entry in fs::read_dir(dir).with_context(|| format!("无法读取目录 {}", dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if current_executable
            .as_ref()
            .is_some_and(|current| entry.path().canonicalize().ok().as_ref() == Some(current))
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_installer_filename(&name) {
            installers.push(entry.path());
        }
    }
    installers.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
    });
    Ok(installers)
}

fn is_installer_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".exe") else {
        return false;
    };
    stem.split_once("_xiaomipcmanager_")
        .is_some_and(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
}

/// 在安装包同目录释放内嵌 `msimg32.dll`。
pub fn prepare_installer(installer: &Path) -> Result<PathBuf> {
    if !installer.is_file() {
        bail!("指定的安装包不存在：{}", installer.display());
    }
    let is_exe = installer
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_exe {
        bail!("安装包必须是 .exe 文件：{}", installer.display());
    }
    let parent = installer.parent().context("无法确定安装包所在目录")?;
    crate::device_spoof::deploy_proxy(parent)
}

/// 返回 Patcher 可执行文件所在目录。
pub fn patcher_dir() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("无法确定 Patcher 可执行文件路径")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("无法确定 Patcher 所在目录")
}

/// 从 HTTP(S) URL 推导安全的本地安装包文件名。
pub fn download_filename(url: &str) -> Result<String> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        bail!("下载地址必须使用 http:// 或 https://");
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let candidate = without_query.rsplit('/').next().unwrap_or_default();
    let filename = Path::new(candidate)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.to_ascii_lowercase().ends_with(".exe")
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        })
        .unwrap_or("XiaomiPCManagerInstaller.exe");
    Ok(filename.to_string())
}

/// 使用 Windows PowerShell 的 Invoke-WebRequest 将安装包下载到 Patcher 同目录。
pub fn download_installer(url: &str, target_dir: &Path) -> Result<PathBuf> {
    let target = target_dir.join(download_filename(url)?);
    if target.exists() {
        bail!("下载目标已存在，为避免覆盖已取消：{}", target.display());
    }
    let mut temporary_name = target.as_os_str().to_os_string();
    temporary_name.push(".download.tmp");
    let temporary = PathBuf::from(temporary_name);
    if temporary.exists() {
        fs::remove_file(&temporary)
            .with_context(|| format!("无法清理临时下载文件 {}", temporary.display()))?;
    }

    // URL 和目标路径通过环境变量传递，避免将用户输入拼接进 PowerShell 脚本。
    let script = "$ErrorActionPreference = 'Stop'; \
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
        Invoke-WebRequest -UseBasicParsing -Uri $env:MIPCM_DOWNLOAD_URL -OutFile $env:MIPCM_DOWNLOAD_TARGET";
    let powershell = system_powershell_path()?;
    let status = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MIPCM_DOWNLOAD_URL", url)
        .env("MIPCM_DOWNLOAD_TARGET", &temporary)
        .status()
        .context("无法启动 Windows PowerShell 下载安装包")?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        bail!("Windows PowerShell 下载失败（退出码：{status}）");
    }
    fs::rename(&temporary, &target)
        .with_context(|| format!("无法将下载文件保存为 {}", target.display()))?;
    Ok(target)
}

#[cfg(windows)]
fn system_powershell_path() -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: buffer 指向可写的 u16 数组，长度以 u32 准确传入。
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(std::io::Error::last_os_error()).context("无法获取 Windows 系统目录");
    }
    if length as usize >= buffer.len() {
        bail!("Windows 系统目录路径过长");
    }
    let windows_dir = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    let powershell = windows_dir
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !powershell.is_file() {
        bail!("未找到系统 Windows PowerShell：{}", powershell.display());
    }
    Ok(powershell)
}

#[cfg(not(windows))]
fn system_powershell_path() -> Result<PathBuf> {
    bail!("安装包下载仅支持 Windows PowerShell")
}

/// 释放代理 DLL 后启动安装包，返回子进程 PID。
pub fn launch_installer(installer: &Path) -> Result<u32> {
    let installer = installer
        .canonicalize()
        .with_context(|| format!("无法解析安装包路径 {}", installer.display()))?;
    let parent = installer.parent().context("无法确定安装包所在目录")?;
    let proxy = parent.join(crate::device_spoof::PROXY_DLL_NAME);
    let previous_proxy = match fs::read(&proxy) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context(format!("无法读取 {}", proxy.display())),
    };
    let backup = crate::install::backup_path(&proxy);
    let backup_existed = backup.exists();
    let temporary = patch_temporary_path(&proxy);
    let temporary_existed = temporary.exists();
    if let Err(error) = prepare_installer(&installer) {
        let deploy_error = error.context(format!("无法为安装包部署 {}", proxy.display()));
        return Err(rollback_after_error(
            deploy_error,
            &proxy,
            previous_proxy.as_deref(),
            &backup,
            backup_existed,
            &temporary,
            temporary_existed,
        ));
    }

    match Command::new(&installer).current_dir(parent).spawn() {
        Ok(child) => Ok(child.id()),
        Err(error) => {
            let launch_error =
                anyhow!(error).context(format!("无法启动安装包 {}", installer.display()));
            Err(rollback_after_error(
                launch_error,
                &proxy,
                previous_proxy.as_deref(),
                &backup,
                backup_existed,
                &temporary,
                temporary_existed,
            ))
        }
    }
}

fn rollback_after_error(
    error: anyhow::Error,
    proxy: &Path,
    previous: Option<&[u8]>,
    backup: &Path,
    backup_existed: bool,
    temporary: &Path,
    temporary_existed: bool,
) -> anyhow::Error {
    match rollback_proxy(
        proxy,
        previous,
        backup,
        backup_existed,
        temporary,
        temporary_existed,
    ) {
        Ok(()) => error,
        Err(rollback_error) => error.context(format!(
            "回滚 {} 也失败：{rollback_error:#}",
            proxy.display()
        )),
    }
}

fn rollback_proxy(
    proxy: &Path,
    previous: Option<&[u8]>,
    backup: &Path,
    backup_existed: bool,
    temporary: &Path,
    temporary_existed: bool,
) -> Result<()> {
    if let Some(bytes) = previous {
        fs::write(proxy, bytes).with_context(|| format!("无法恢复 {}", proxy.display()))?;
    } else if proxy.exists() {
        fs::remove_file(proxy).with_context(|| format!("无法移除 {}", proxy.display()))?;
    }
    if !backup_existed && backup.exists() {
        fs::remove_file(backup).with_context(|| format!("无法移除 {}", backup.display()))?;
    }
    if !temporary_existed && temporary.is_file() {
        fs::remove_file(temporary).with_context(|| format!("无法移除 {}", temporary.display()))?;
    }
    Ok(())
}

fn patch_temporary_path(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_os_string();
    path.push(".patch.tmp");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mipcm_installer_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_only_xiaomi_pc_manager_installer_executables() {
        let dir = fixture_dir("discovery");
        let expected = [
            "AAA_XiaomiPCManager_stable_5.8.0.75.exe",
            "NeR5_XiaomiPCManager_feature_p52_5.8.0.74_9900fa23.exe",
        ];
        for name in expected {
            fs::write(dir.join(name), b"fixture").unwrap();
        }
        fs::write(dir.join("XiaomiPCManager.exe"), b"fixture").unwrap();
        fs::write(
            dir.join("NeR5_XiaomiPCManager_feature_p52_5.8.0.74.txt"),
            b"fixture",
        )
        .unwrap();
        fs::create_dir(dir.join("Fake_XiaomiPCManager_folder.exe")).unwrap();

        let found = find_local_installers(&dir).unwrap();

        assert_eq!(
            found,
            expected.map(|name| dir.join(name)).to_vec(),
            "安装包应按文件名稳定排序"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepares_embedded_proxy_next_to_installer() {
        let dir = fixture_dir("proxy");
        let installer = dir.join("NeR5_XiaomiPCManager_feature_5.8.0.74.exe");
        fs::write(&installer, b"fixture").unwrap();

        let proxy = prepare_installer(&installer).unwrap();

        assert_eq!(proxy, dir.join(crate::device_spoof::PROXY_DLL_NAME));
        assert!(crate::device_spoof::proxy_is_current(&dir));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn derives_safe_download_name_from_http_url() {
        assert_eq!(
            download_filename(
                "https://example.com/releases/NeR5_XiaomiPCManager_feature_5.8.0.74.exe?token=abc"
            )
            .unwrap(),
            "NeR5_XiaomiPCManager_feature_5.8.0.74.exe"
        );
        assert_eq!(
            download_filename("https://example.com/download?id=123").unwrap(),
            "XiaomiPCManagerInstaller.exe"
        );
        assert!(download_filename("ftp://example.com/setup.exe").is_err());
    }

    #[test]
    fn downloads_installer_with_windows_powershell() {
        let dir = fixture_dir("download");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload",
                )
                .unwrap();
        });
        let url = format!("http://{address}/Test_XiaomiPCManager_feature_5.8.0.74.exe");

        let downloaded = download_installer(&url, &dir).unwrap();

        server.join().unwrap();
        assert_eq!(fs::read(downloaded).unwrap(), b"payload");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn resolves_powershell_from_the_windows_system_directory() {
        let powershell = system_powershell_path().unwrap();
        assert!(powershell.is_absolute());
        assert_eq!(
            powershell.file_name().unwrap().to_string_lossy(),
            "powershell.exe"
        );
        assert!(powershell.is_file());
    }

    #[test]
    fn restores_existing_proxy_when_installer_fails_to_launch() {
        let dir = fixture_dir("launch_rollback");
        let installer = dir.join("Broken_XiaomiPCManager_fixture.exe");
        let proxy = dir.join(crate::device_spoof::PROXY_DLL_NAME);
        fs::write(&installer, b"not a Windows executable").unwrap();
        fs::write(&proxy, b"original proxy").unwrap();

        assert!(launch_installer(&installer).is_err());

        assert_eq!(fs::read(&proxy).unwrap(), b"original proxy");
        assert!(!crate::install::backup_path(&proxy).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restores_existing_proxy_when_proxy_deployment_fails() {
        let dir = fixture_dir("deploy_rollback");
        let installer = dir.join("Broken_XiaomiPCManager_fixture.exe");
        let proxy = dir.join(crate::device_spoof::PROXY_DLL_NAME);
        let temporary = patch_temporary_path(&proxy);
        fs::write(&installer, b"not a Windows executable").unwrap();
        fs::write(&proxy, b"original proxy").unwrap();
        fs::create_dir(&temporary).unwrap();

        assert!(launch_installer(&installer).is_err());

        assert_eq!(fs::read(&proxy).unwrap(), b"original proxy");
        assert!(!crate::install::backup_path(&proxy).exists());
        assert!(temporary.is_dir(), "预先存在的临时路径不应被删除");
        fs::remove_dir_all(dir).unwrap();
    }
}
