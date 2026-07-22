use crate::{infra, install};
use anyhow::{Result, bail};
use std::path::Path;

/// 代理 DLL 文件名。
pub const PROXY_DLL_NAME: &str = "msimg32.dll";

/// 内嵌的代理 DLL 字节（编译期打入二进制）。
const EMBEDDED_MSIMG32: &[u8] = include_bytes!("dlls/msimg32.dll");

/// 伪装机型写入的注册表位置。
const REG_SUBKEY: &str = r"Software\SmartSharePatch";
const REG_VALUE: &str = "SpoofDevice";

/// 默认机型。
pub const DEFAULT_MODEL: &str = "TM2424";

/// 预置机型。
pub struct ModelPreset {
    pub code: &'static str,
    pub name: &'static str,
}

pub const PRESETS: &[ModelPreset] = &[
    ModelPreset {
        code: "TM2424",
        name: "Xiaomi Book Pro 14 (2026)",
    },
    ModelPreset {
        code: "TM2309",
        name: "Redmi Book 16 (2024)",
    },
];

/// 应用设备伪装：释放代理 DLL 到版本目录，并写入注册表机型。
pub fn apply(version_dir: &Path, model: &str) -> Result<()> {
    if model.trim().is_empty() {
        bail!("机型代号不能为空");
    }
    deploy_proxy(version_dir)?;
    set_registry(model.trim())?;
    Ok(())
}

/// 仅写入伪装机型注册表（安装包启动前用于绕过「暂不支持本设备」）。
pub fn ensure_spoof_model(model: &str) -> Result<()> {
    if model.trim().is_empty() {
        bail!("机型代号不能为空");
    }
    set_registry(model.trim())
}

/// 将内嵌代理 DLL 释放到指定目录，必要时备份已有文件。
pub fn deploy_proxy(target_dir: &Path) -> Result<std::path::PathBuf> {
    let target = target_dir.join(PROXY_DLL_NAME);

    // 若目标已存在且并非我们的 DLL，则先备份（避免覆盖系统/他人文件）。
    if target.exists() {
        let cur = std::fs::read(&target).unwrap_or_default();
        if cur != EMBEDDED_MSIMG32 {
            let bak = install::backup_path(&target);
            if !bak.exists() {
                std::fs::copy(&target, &bak)?;
            }
        }
    }
    install::write_file_atomic(&target, EMBEDDED_MSIMG32)?;
    Ok(target)
}

/// 还原设备伪装：移除注册表机型，删除我们释放的代理 DLL（或还原原文件）。
pub fn revert(version_dir: &Path) -> Result<()> {
    remove_registry()?;
    let target = version_dir.join(PROXY_DLL_NAME);
    let bak = install::backup_path(&target);
    if bak.exists() {
        std::fs::copy(&bak, &target)?;
        std::fs::remove_file(&bak)?;
    } else if target.exists() {
        let cur = std::fs::read(&target).unwrap_or_default();
        if cur == EMBEDDED_MSIMG32 {
            std::fs::remove_file(&target)?;
        }
    }
    Ok(())
}

/// 读取当前状态：(代理DLL是否就位, 注册表机型)。用于 status 展示。
pub fn current_state(version_dir: &Path) -> (bool, Option<String>) {
    (proxy_is_current(version_dir), read_registry())
}

/// 检查指定目录中是否已是内嵌代理 DLL。
pub fn proxy_is_current(dir: &Path) -> bool {
    let target = dir.join(PROXY_DLL_NAME);
    std::fs::read(&target)
        .map(|c| c == EMBEDDED_MSIMG32)
        .unwrap_or(false)
}

fn set_registry(model: &str) -> Result<()> {
    infra::registry::set_hkcu_string(REG_SUBKEY, REG_VALUE, model)
}

fn remove_registry() -> Result<()> {
    infra::registry::delete_hkcu_value(REG_SUBKEY, REG_VALUE)
}

fn read_registry() -> Option<String> {
    infra::registry::get_hkcu_string(REG_SUBKEY, REG_VALUE)
}
