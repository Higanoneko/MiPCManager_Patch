//! 小米电脑管家安装目录定位、版本目录解析、进程关闭与文件备份/还原。

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// 默认安装根目录。
pub const DEFAULT_INSTALL_ROOT: &str = r"C:\Program Files\MI\XiaomiPCManager";

/// 探测安装根目录。
pub fn find_install_root() -> Option<PathBuf> {
    let default = PathBuf::from(DEFAULT_INSTALL_ROOT);
    if default.is_dir() {
        return Some(default);
    }
    // 退而求其次：%ProgramFiles%\MI\XiaomiPCManager
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let p = Path::new(&pf).join("MI").join("XiaomiPCManager");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// 在安装根目录下找出版本号最高的版本目录（形如 `5.8.0.14`）。
pub fn latest_version_dir(root: &Path) -> Result<PathBuf> {
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in fs::read_dir(root).with_context(|| format!("无法读取目录 {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(ver) = parse_version(&name) {
            let take = match &best {
                Some((b, _)) => &ver > b,
                None => true,
            };
            if take {
                best = Some((ver, entry.path()));
            }
        }
    }
    best.map(|(_, p)| p)
        .with_context(|| format!("在 {} 下未找到任何版本目录", root.display()))
}

/// 解析形如 `5.8.0.14` 的版本号为数字数组；非纯版本号返回 None。
fn parse_version(s: &str) -> Option<Vec<u64>> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<u64> = s
        .split('.')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<_>>()?;
    if parts.is_empty() { None } else { Some(parts) }
}

/// 关闭小米电脑管家相关进程。
///
/// 优先关闭安装目录内运行的进程，再用已知进程名兜底，避免版本更新新增子进程后漏关。
#[cfg(windows)]
pub fn kill_mipcmanager_processes(known_names: &[&str]) -> usize {
    kill_processes(known_names, find_install_root().as_deref())
}

#[cfg(not(windows))]
pub fn kill_mipcmanager_processes(_known_names: &[&str]) -> usize {
    0
}

/// 按进程名（不含扩展名，大小写不敏感）结束进程。返回被结束的进程数。
#[cfg(windows)]
pub fn kill_by_names(names: &[&str]) -> usize {
    kill_processes(names, None)
}

#[cfg(windows)]
fn kill_processes(names: &[&str], install_root: Option<&Path>) -> usize {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let current_pid = sysinfo::get_current_pid().ok();
    let mut killed = 0;
    for proc in sys.processes().values() {
        if Some(proc.pid()) == current_pid {
            continue;
        }
        let pname = proc.name().to_string_lossy();
        let stem = pname.strip_suffix(".exe").unwrap_or(&pname);
        let known_name = names.iter().any(|n| n.eq_ignore_ascii_case(stem));
        let in_install_root = install_root
            .and_then(|root| proc.exe().map(|exe| path_is_under(exe, root)))
            .unwrap_or(false);
        if (known_name || in_install_root) && proc.kill() {
            killed += 1;
        }
    }
    killed
}

#[cfg(windows)]
fn path_is_under(path: &Path, root: &Path) -> bool {
    let path = normalize_process_path(path);
    let root = normalize_process_path(root);
    path == root || path.starts_with(&format!("{root}\\"))
}

#[cfg(windows)]
fn normalize_process_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(not(windows))]
pub fn kill_by_names(_names: &[&str]) -> usize {
    0
}

/// 按进程名关闭并等待进程退出。返回关闭数量与最终仍在运行的进程名。
#[cfg(windows)]
pub fn kill_by_names_until_gone(
    names: &[&str],
    timeout: std::time::Duration,
) -> (usize, Vec<String>) {
    let deadline = std::time::Instant::now() + timeout;
    let mut killed = 0;
    loop {
        killed += kill_by_names(names);
        let running = running_by_names(names);
        if running.is_empty() || std::time::Instant::now() >= deadline {
            return (killed, running);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[cfg(not(windows))]
pub fn kill_by_names_until_gone(
    _names: &[&str],
    _timeout: std::time::Duration,
) -> (usize, Vec<String>) {
    (0, Vec::new())
}

/// 返回仍在运行的指定进程名（不含扩展名，大小写不敏感）。
#[cfg(windows)]
pub fn running_by_names(names: &[&str]) -> Vec<String> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let current_pid = sysinfo::get_current_pid().ok();
    let mut running = Vec::new();
    for proc in sys.processes().values() {
        if Some(proc.pid()) == current_pid {
            continue;
        }
        let pname = proc.name().to_string_lossy();
        let stem = pname.strip_suffix(".exe").unwrap_or(&pname);
        if names.iter().any(|n| n.eq_ignore_ascii_case(stem)) {
            running.push(format!("{stem}.exe"));
        }
    }
    running.sort();
    running.dedup();
    running
}

#[cfg(not(windows))]
pub fn running_by_names(_names: &[&str]) -> Vec<String> {
    Vec::new()
}

/// 为目标文件创建备份 `<file>.orig.bak`。若备份已存在则保留（视其为最初的原始文件）。
pub fn ensure_backup(file: &Path) -> Result<PathBuf> {
    let bak = backup_path(file);
    if !bak.exists() {
        fs::copy(file, &bak)
            .with_context(|| format!("备份失败 {} -> {}", file.display(), bak.display()))?;
    }
    Ok(bak)
}

/// 从备份还原目标文件。
pub fn restore_backup(file: &Path) -> Result<()> {
    let bak = backup_path(file);
    if !bak.exists() {
        bail!("未找到备份文件 {}", bak.display());
    }
    fs::copy(&bak, file)
        .with_context(|| format!("还原失败 {} -> {}", bak.display(), file.display()))?;
    Ok(())
}

/// 备份文件路径：在原文件名后追加 `.orig.bak`。
pub fn backup_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".orig.bak");
    PathBuf::from(s)
}

/// 原子写回：先写到同目录临时文件再替换，降低写坏风险。
pub fn write_file_atomic(file: &Path, data: &[u8]) -> Result<()> {
    let tmp = {
        let mut s = file.as_os_str().to_os_string();
        s.push(".patch.tmp");
        PathBuf::from(s)
    };
    fs::write(&tmp, data).with_context(|| format!("写入临时文件失败 {}", tmp.display()))?;
    // Windows 上 rename 覆盖已存在文件会失败，先删原文件。
    if file.exists() {
        fs::remove_file(file).with_context(|| format!("删除旧文件失败 {}", file.display()))?;
    }
    fs::rename(&tmp, file).with_context(|| format!("替换文件失败 {}", file.display()))?;
    Ok(())
}
