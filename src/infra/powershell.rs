//! Windows PowerShell 调用辅助。
//!
//! 为各业务模块提供统一的 `run_powershell` 入口，避免跨模块重复实现。

use anyhow::{Context, Result, bail};
use std::process::Command;

/// 执行 PowerShell 命令并返回 stdout。
///
/// 非零退出码时：若 stdout 有内容则返回 stdout（部分 cmdlet 如
/// `Get-AppxPackage` 可能同时输出有效数据与警告），否则 bail。
#[cfg(windows)]
pub fn run_powershell(script: &str) -> Result<String> {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .context("无法启动 PowerShell")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let trimmed_stdout = stdout.trim();
        let trimmed_stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !trimmed_stdout.is_empty() {
            return Ok(stdout);
        }
        if !trimmed_stderr.is_empty() {
            bail!("PowerShell 执行失败：{trimmed_stderr}");
        }
        bail!("PowerShell 执行失败（无输出）");
    }
    Ok(stdout)
}

#[cfg(not(windows))]
pub fn run_powershell(_script: &str) -> Result<String> {
    bail!("PowerShell 仅支持 Windows")
}
