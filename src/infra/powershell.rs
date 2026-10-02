//! Windows PowerShell 调用辅助。
//!
//! 为各业务模块提供固定系统路径、统一 UTF-8 输出与本机代码页回退。

#[cfg(windows)]
use anyhow::Context;
use anyhow::{Result, bail};
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use windows_sys::Win32::Globalization::{GetOEMCP, MultiByteToWideChar};
#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[cfg(windows)]
const UTF8_OUTPUT_COMMAND: &str = "$OutputEncoding = [Console]::OutputEncoding = \
    [System.Text.UTF8Encoding]::new($false); \
    & ([ScriptBlock]::Create($env:MIPCM_POWERSHELL_SCRIPT))";

/// 执行 PowerShell 命令并返回 stdout。
/// 保留项目既有兼容行为：非零退出码时，若 stdout 有内容则返回 stdout。
#[cfg(windows)]
pub fn run_powershell(script: &str) -> Result<String> {
    run_powershell_impl(script, &[], false)
}

/// 严格执行 PowerShell 命令。任何非零退出码都视为失败。
#[cfg(windows)]
pub fn run_powershell_strict(script: &str) -> Result<String> {
    run_powershell_with_env_strict(script, &[])
}

/// 严格执行 PowerShell 命令，并将动态数据作为环境变量传入。
///
/// 路径、URL 等外部数据不应插值到 PowerShell 源码中；通过环境变量传递可避免
/// 引号、反引号、`$()` 等字符参与脚本解析。
///
/// 任何非零退出码都返回错误，同时保留 stdout/stderr 便于诊断。
#[cfg(windows)]
pub fn run_powershell_with_env_strict(script: &str, env_vars: &[(&str, &OsStr)]) -> Result<String> {
    run_powershell_impl(script, env_vars, true)
}

#[cfg(windows)]
fn run_powershell_impl(script: &str, env_vars: &[(&str, &OsStr)], strict: bool) -> Result<String> {
    let powershell = system_powershell_path()?;
    let mut command = Command::new(&powershell);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            UTF8_OUTPUT_COMMAND,
        ])
        .env("MIPCM_POWERSHELL_SCRIPT", script);
    for (name, value) in env_vars {
        command.env(name, value);
    }
    let output = command
        .output()
        .with_context(|| format!("无法启动系统 Windows PowerShell：{}", powershell.display()))?;
    let stdout = decode_powershell_output(&output.stdout);
    if !output.status.success() {
        let trimmed_stdout = stdout.trim();
        if !strict && !trimmed_stdout.is_empty() {
            return Ok(stdout);
        }
        let stderr = decode_powershell_output(&output.stderr);
        let trimmed_stderr = stderr.trim();
        match (trimmed_stdout.is_empty(), trimmed_stderr.is_empty()) {
            (false, false) => {
                bail!("PowerShell 执行失败；stdout: {trimmed_stdout}；stderr: {trimmed_stderr}")
            }
            (false, true) => bail!("PowerShell 执行失败；stdout: {trimmed_stdout}"),
            (true, false) => bail!("PowerShell 执行失败：{trimmed_stderr}"),
            (true, true) => bail!("PowerShell 执行失败（无输出）"),
        }
    }
    Ok(stdout)
}

/// 返回系统自带的 Windows PowerShell 5.1 路径，不依赖 `PATH`，也不会选择 `pwsh.exe`。
#[cfg(windows)]
pub(crate) fn system_powershell_path() -> Result<PathBuf> {
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

#[cfg(windows)]
fn decode_powershell_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }

    // Windows PowerShell 5.1 在 UTF-8 前导脚本执行前发生解析错误时，仍可能按
    // 控制台 OEM 代码页写入管道，因此保留本机代码页回退。
    // SAFETY: GetOEMCP 不接收参数，也不要求调用方维持额外不变量。
    let oem_code_page = unsafe { GetOEMCP() };
    decode_windows_code_page(bytes, oem_code_page)
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(windows)]
fn decode_windows_code_page(bytes: &[u8], code_page: u32) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let byte_count = i32::try_from(bytes.len()).ok()?;
    // SAFETY: bytes 在调用期间有效；第一次调用只查询所需 UTF-16 长度。
    let wide_count = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            byte_count,
            std::ptr::null_mut(),
            0,
        )
    };
    if wide_count == 0 {
        return None;
    }

    let mut wide = vec![0_u16; wide_count as usize];
    // SAFETY: wide 已按上一次调用返回的长度分配，两个切片在调用期间均有效。
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            byte_count,
            wide.as_mut_ptr(),
            wide_count,
        )
    };
    if written == 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(not(windows))]
pub fn run_powershell(_script: &str) -> Result<String> {
    bail!("PowerShell 仅支持 Windows")
}

#[cfg(not(windows))]
pub fn run_powershell_strict(_script: &str) -> Result<String> {
    bail!("PowerShell 仅支持 Windows")
}

#[cfg(not(windows))]
pub fn run_powershell_with_env_strict(
    _script: &str,
    _env_vars: &[(&str, &OsStr)],
) -> Result<String> {
    bail!("PowerShell 仅支持 Windows")
}

#[cfg(not(windows))]
pub(crate) fn system_powershell_path() -> Result<PathBuf> {
    bail!("系统 Windows PowerShell 仅支持 Windows")
}

#[cfg(all(test, windows))]
mod tests {
    use super::{decode_windows_code_page, run_powershell, run_powershell_strict};

    #[test]
    fn decodes_cp936_output() {
        let encoded = [177, 224, 194, 235, 178, 226, 202, 212];
        assert_eq!(
            decode_windows_code_page(&encoded, 936).as_deref(),
            Some("编码测试")
        );
    }

    #[test]
    fn strict_execution_rejects_nonzero_exit_with_stdout() {
        let error = run_powershell_strict("'partial'; throw 'failed'").unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("partial"));
        assert!(message.contains("failed"));
    }

    #[test]
    fn legacy_execution_preserves_stdout_on_nonzero_exit() {
        let output = run_powershell("'partial'; throw 'failed'").unwrap();
        assert_eq!(output.trim(), "partial");
    }
}
