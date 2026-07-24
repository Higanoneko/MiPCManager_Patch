//! MiDrop Ext MSIX 包卸载、产品卸载（MiPCManager / PcContinuity）、服务清理、文件清理。
//!
//! 卸载为不可逆操作，调用方需在执行前获取用户确认。

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// MiDrop Ext MSIX 包名称。
const MSIX_PACKAGE_NAME: &str = "5f71dad9-3e77-4ada-9fad-12c2e761288f";

/// PcContinuity 相关服务。
const PC_CONTINUITY_SERVICES: &[&str] = &["micont_service", "MiPcContinuityService"];

/// MiPCManager 相关服务（包含 PcContinuity 共用项）。
const MIPC_MANAGER_SERVICES: &[&str] = &[
    "AIService",
    "MAFSvr",
    "MiDeviceService",
    "MiPlayCastService",
    "MiService",
    "micont_service",
    "dist_service",
    "DistributedService",
    "handoff_service",
];

use crate::infra::powershell::run_powershell;

// ── MSIX 包检测与卸载 ──

/// 检测 MiDrop Ext MSIX 包是否已安装。
/// 返回 `Some(PackageFullName)` 或 `None`。
#[cfg(windows)]
pub fn detect_msix() -> Result<Option<String>> {
    let script = format!(
        "Get-AppxPackage -Name '{MSIX_PACKAGE_NAME}' | Select-Object -ExpandProperty PackageFullName"
    );
    let output = run_powershell(&script)?;
    let name = output.trim().to_string();
    if name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(name))
    }
}

/// 按 PackageFullName 卸载 MSIX 包。
pub fn remove_msix(package_full_name: &str) -> Result<()> {
    let script = format!("Remove-AppxPackage -Package '{package_full_name}'");
    run_powershell(&script)?;
    Ok(())
}

// ── 资源管理器重启 ──

/// 重启 Windows 资源管理器（explorer.exe）。
#[cfg(windows)]
pub fn restart_explorer() -> Result<()> {
    Command::new("taskkill")
        .args(["/f", "/im", "explorer.exe"])
        .status()
        .context("无法结束 explorer.exe")?;
    // 短暂等待确保进程退出
    std::thread::sleep(std::time::Duration::from_millis(500));
    Command::new("explorer.exe")
        .spawn()
        .context("无法启动 explorer.exe")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn restart_explorer() -> Result<()> {
    bail!("资源管理器重启仅支持 Windows")
}

// ── 服务管理 ──

/// 检查 Windows 服务是否存在。
pub fn service_exists(name: &str) -> Result<bool> {
    let script = format!(
        "Get-Service -Name '{name}' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name"
    );
    match run_powershell(&script) {
        Ok(output) => Ok(!output.trim().is_empty()),
        Err(_) => Ok(false),
    }
}

/// 删除 Windows 服务。返回 `Ok(true)` 表示已删除，`Ok(false)` 表示服务不存在。
pub fn remove_service(name: &str) -> Result<bool> {
    if !service_exists(name)? {
        return Ok(false);
    }
    let script = format!("Remove-Service -Name '{name}'");
    run_powershell(&script)?;
    Ok(true)
}

// ── 产品卸载 ──

/// 运行产品自带的 uninstall.exe，等待退出，验证是否成功删除自身。
/// 返回 `true` 表示 uninstall.exe 已被删除（卸载成功）。
pub fn run_product_uninstaller(uninstall_exe: &Path) -> Result<bool> {
    if !uninstall_exe.is_file() {
        bail!("未找到卸载程序：{}", uninstall_exe.display());
    }
    let status = Command::new(uninstall_exe)
        .spawn()
        .with_context(|| format!("无法启动卸载程序：{}", uninstall_exe.display()))?
        .wait()
        .context("等待卸载程序退出时出错")?;
    // 退出码非零不一定是失败；以文件是否被删除为准。
    let _ = status;
    Ok(!uninstall_exe.is_file())
}

/// 删除目录（若存在）。返回 `Ok(true)` 表示已删除，`Ok(false)` 表示不存在。
pub fn remove_dir_if_exists(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(path)
        .with_context(|| format!("无法删除目录：{}", path.display()))?;
    Ok(true)
}

// ── 完整卸载流程 ──

/// 卸载小米电脑管家（完整流程）。
pub fn uninstall_xiaomi_pc_manager(root: &Path, log: &mut Vec<String>) -> Result<()> {
    log.push(format!("开始卸载小米电脑管家：{}", root.display()));

    // 1. 找到版本目录并运行 uninstall.exe
    let version = crate::install::latest_version_dir(root)?;
    let uninstall_exe = version.join("uninstall.exe");
    log.push(format!(
        "  正在运行卸载程序：{}",
        uninstall_exe.display()
    ));
    let removed = run_product_uninstaller(&uninstall_exe)?;
    if !removed {
        log.push(format!(
            "  ⚠ 卸载程序未删除自身，卸载可能未完成：{}",
            uninstall_exe.display()
        ));
    } else {
        log.push("  ✓ 主程序卸载完成".to_string());
    }

    // 2. 卸载 AIService
    uninstall_sub_product(log, r"C:\Program Files\MI\AIService", "AIService");

    // 3. 卸载 MiService
    uninstall_sub_product(
        log,
        r"C:\Program Files (x86)\Timi Personal Computing\MiService",
        "MiService",
    );

    // 4. 删除服务
    log.push("  正在移除服务…".to_string());
    for name in MIPC_MANAGER_SERVICES {
        match remove_service(name) {
            Ok(true) => log.push(format!("    ✓ 已删除服务：{name}")),
            Ok(false) => {} // 服务不存在，跳过
            Err(e) => log.push(format!("    ⚠ 删除服务 {name} 失败：{e}")),
        }
    }

    // 5. 清理目录
    log.push("  正在清理文件…".to_string());
    cleanup_product_directories(log, true);

    log.push("✓ 小米电脑管家卸载完成".to_string());
    Ok(())
}

/// 卸载小米互联 / PcContinuity（完整流程）。
pub fn uninstall_pc_continuity(root: &Path, log: &mut Vec<String>) -> Result<()> {
    log.push(format!(
        "开始卸载小米互联 / PcContinuity：{}",
        root.display()
    ));

    // 1. 找到版本目录并运行 uninstall.exe
    let version = crate::install::latest_version_dir(root)?;
    let uninstall_exe = version.join("uninstall.exe");
    log.push(format!(
        "  正在运行卸载程序：{}",
        uninstall_exe.display()
    ));
    let removed = run_product_uninstaller(&uninstall_exe)?;
    if !removed {
        log.push(format!(
            "  ⚠ 卸载程序未删除自身，卸载可能未完成：{}",
            uninstall_exe.display()
        ));
    } else {
        log.push("  ✓ 主程序卸载完成".to_string());
    }

    // 2. 删除服务
    log.push("  正在移除服务…".to_string());
    for name in PC_CONTINUITY_SERVICES {
        match remove_service(name) {
            Ok(true) => log.push(format!("    ✓ 已删除服务：{name}")),
            Ok(false) => {}
            Err(e) => log.push(format!("    ⚠ 删除服务 {name} 失败：{e}")),
        }
    }

    // 3. 清理目录
    log.push("  正在清理文件…".to_string());
    cleanup_product_directories(log, false);

    log.push("✓ 小米互联 / PcContinuity 卸载完成".to_string());
    Ok(())
}

/// 卸载子产品（AIService / MiService）。
fn uninstall_sub_product(log: &mut Vec<String>, root_path: &str, label: &str) {
    let root = Path::new(root_path);
    if !root.is_dir() {
        log.push(format!("  - {label} 未安装，跳过"));
        return;
    }
    match crate::install::latest_version_dir(root) {
        Ok(version) => {
            let exe = version.join("uninstall.exe");
            if exe.is_file() {
                log.push(format!("  正在卸载 {label}：{}", exe.display()));
                match run_product_uninstaller(&exe) {
                    Ok(true) => log.push(format!("    ✓ {label} 卸载完成")),
                    Ok(false) => log.push(format!(
                        "    ⚠ {label} 卸载程序未删除自身，卸载可能未完成"
                    )),
                    Err(e) => log.push(format!("    ⚠ 卸载 {label} 失败：{e}")),
                }
            } else {
                log.push(format!(
                    "    ⚠ {label} 目录存在但未找到 uninstall.exe"
                ));
            }
        }
        Err(_) => {
            log.push(format!(
                "    ⚠ {label} 目录存在但无版本子目录"
            ));
        }
    }
}

/// 清理产品相关目录和临时文件。
fn cleanup_product_directories(log: &mut Vec<String>, is_manager: bool) {
    // C:\ProgramData\MI
    match remove_dir_if_exists(Path::new(r"C:\ProgramData\MI")) {
        Ok(true) => log.push("    ✓ 已删除 C:\\ProgramData\\MI".to_string()),
        Ok(false) => {}
        Err(e) => log.push(format!(
            "    ⚠ 清理 C:\\ProgramData\\MI 失败：{e}"
        )),
    }

    // %LOCALAPPDATA%\Temp\Timi Personal Computing\
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if !localappdata.is_empty() {
        let timi_temp = Path::new(&localappdata)
            .join("Temp")
            .join("Timi Personal Computing");

        let subdirs: &[&str] = if is_manager {
            &["MiServiceTMPX", "XiaomiPCManagerTMPX", "AIServiceTMPX"]
        } else {
            &["PcContinuityTMPX"]
        };

        for sub in subdirs {
            let dir = timi_temp.join(sub);
            match remove_dir_if_exists(&dir) {
                Ok(true) => log.push(format!("    ✓ 已删除 {}", dir.display())),
                Ok(false) => {}
                Err(e) => log.push(format!(
                    "    ⚠ 清理 {} 失败：{e}",
                    dir.display()
                )),
            }
        }

        // 如果 Timi Personal Computing 目录变空，也删除它
        if timi_temp.is_dir() {
            let _ = remove_dir_if_exists(&timi_temp);
        }
    }
}

/// 获取卸载描述文本（用于前端确认提示）。
pub fn uninstall_description() -> Result<String> {
    let manager_root = crate::install::find_install_root();
    let continuity_root = crate::install::find_pc_continuity_root();

    match (manager_root, continuity_root) {
        (Some(_), Some(_)) => {
            bail!(
                "同时检测到小米电脑管家和小米互联。\n当前不支持同时安装，请逐一卸载。"
            )
        }
        (Some(root), None) => Ok(format!(
            "将卸载 小米电脑管家\n\n安装目录：{}\n包含：主程序、AIService、MiService\n将删除所有相关服务与临时文件\n\n此操作不可逆！",
            root.display()
        )),
        (None, Some(root)) => Ok(format!(
            "将卸载 小米互联 / PcContinuity\n\n安装目录：{}\n将删除所有相关服务与临时文件\n\n此操作不可逆！",
            root.display()
        )),
        (None, None) => {
            bail!("未检测到已安装的小米电脑管家或小米互联")
        }
    }
}
