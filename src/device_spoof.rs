use crate::install;
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
    let target = version_dir.join(PROXY_DLL_NAME);

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
    set_registry(model.trim())?;
    Ok(())
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
    let target = version_dir.join(PROXY_DLL_NAME);
    let dll_ok = std::fs::read(&target)
        .map(|c| c == EMBEDDED_MSIMG32)
        .unwrap_or(false);
    (dll_ok, read_registry())
}

#[cfg(windows)]
fn set_registry(model: &str) -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_SUBKEY)?;
    key.set_value(REG_VALUE, &model.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn remove_registry() -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(REG_SUBKEY, KEY_ALL_ACCESS) {
        let _ = key.delete_value(REG_VALUE);
    }
    Ok(())
}

#[cfg(windows)]
fn read_registry() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(REG_SUBKEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>(REG_VALUE).ok())
}

#[cfg(not(windows))]
fn set_registry(_model: &str) -> Result<()> {
    Ok(())
}
#[cfg(not(windows))]
fn remove_registry() -> Result<()> {
    Ok(())
}
#[cfg(not(windows))]
fn read_registry() -> Option<String> {
    None
}
