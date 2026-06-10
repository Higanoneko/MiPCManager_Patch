use crate::install;
use anyhow::{Result, bail};
use std::path::Path;

/// 目标 DLL 文件名。
pub const TARGET_DLL: &str = "micont_rtm.dll";

/// 锚点：宽字符串 `Geo\0`（紧邻待改值名之前，确保唯一）。
const ANCHOR_GEO: &[u8] = &[0x47, 0x00, 0x65, 0x00, 0x6F, 0x00, 0x00, 0x00];
/// 原值名：宽字符串 `Name` + 终止符（10 字节）。
const ORIG_NAME: &[u8] = &[0x4E, 0x00, 0x61, 0x00, 0x6D, 0x00, 0x65, 0x00, 0x00, 0x00];
/// 新值名：宽字符串 `XCN` + 终止符 + 补位（10 字节，与原值名等长）。
const PATCHED_NAME: &[u8] = &[0x58, 0x00, 0x43, 0x00, 0x4E, 0x00, 0x00, 0x00, 0x00, 0x00];

/// 注册表中用于伪装的值名与所在键。
const GEO_KEY: &str = r"Control Panel\International\Geo";
const SPOOF_VALUE_NAME: &str = "XCN";

#[derive(Debug, PartialEq, Eq)]
pub enum PatchOutcome {
    Patched,
    AlreadyPatched,
}

/// 在 DLL 字节中查找并应用值名替换。返回是否实际改动。
pub fn patch_bytes(data: &mut [u8]) -> Result<PatchOutcome> {
    let orig_sig = [ANCHOR_GEO, ORIG_NAME].concat();
    let patched_sig = [ANCHOR_GEO, PATCHED_NAME].concat();

    if find(data, &orig_sig).is_some() {
        let pos = find(data, &orig_sig).unwrap();
        let name_at = pos + ANCHOR_GEO.len();
        data[name_at..name_at + PATCHED_NAME.len()].copy_from_slice(PATCHED_NAME);
        return Ok(PatchOutcome::Patched);
    }
    if find(data, &patched_sig).is_some() {
        return Ok(PatchOutcome::AlreadyPatched);
    }
    bail!("未在 {TARGET_DLL} 中找到 `Geo\\Name` 特征，可能版本结构已变更");
}

/// 对安装目录中的 DLL 应用补丁，并写入注册表伪装值。
pub fn apply(dll_path: &Path, region: &str, write_registry: bool) -> Result<PatchOutcome> {
    install::ensure_backup(dll_path)?;
    let mut data = std::fs::read(dll_path)?;
    let outcome = patch_bytes(&mut data)?;
    if outcome == PatchOutcome::Patched {
        install::write_file_atomic(dll_path, &data)?;
    }
    if write_registry {
        set_registry(region)?;
    }
    Ok(outcome)
}

/// 还原 DLL 并移除注册表伪装值。
pub fn revert(dll_path: &Path, remove_registry: bool) -> Result<()> {
    install::restore_backup(dll_path)?;
    if remove_registry {
        remove_registry_value()?;
    }
    Ok(())
}

/// 写入 `HKCU\Control Panel\International\Geo\XCN = <region>`。
#[cfg(windows)]
pub fn set_registry(region: &str) -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (geo, _) = hkcu.create_subkey(GEO_KEY)?;
    geo.set_value(SPOOF_VALUE_NAME, &region.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn remove_registry_value() -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(geo) = hkcu.open_subkey_with_flags(GEO_KEY, winreg::enums::KEY_ALL_ACCESS) {
        let _ = geo.delete_value(SPOOF_VALUE_NAME);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_registry(_region: &str) -> Result<()> {
    Ok(())
}
#[cfg(not(windows))]
fn remove_registry_value() -> Result<()> {
    Ok(())
}

/// 朴素子串查找。
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_then_idempotent() {
        // 构造：前缀 + Geo\0 + Name\0\0 + 后缀
        let mut buf = vec![0xAB; 4];
        buf.extend_from_slice(ANCHOR_GEO);
        buf.extend_from_slice(ORIG_NAME);
        buf.extend_from_slice(&[0xCD; 4]);
        let snapshot_len = buf.len();

        assert_eq!(patch_bytes(&mut buf).unwrap(), PatchOutcome::Patched);
        assert_eq!(buf.len(), snapshot_len, "补丁必须等长，不得移位");
        // Name 已变为 XCN
        let name_at = 4 + ANCHOR_GEO.len();
        assert_eq!(&buf[name_at..name_at + PATCHED_NAME.len()], PATCHED_NAME);
        // 再次执行应识别为已打补丁
        assert_eq!(patch_bytes(&mut buf).unwrap(), PatchOutcome::AlreadyPatched);
    }
}
