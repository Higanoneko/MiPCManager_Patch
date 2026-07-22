//! Windows 注册表读写封装。
//!
//! 统一替代各补丁模块（locale、device）中各自的 `winreg` 样板代码。
//!
//! 所有函数均为 `#[cfg(windows)]`，非 Windows 目标为空实现。

/// 在 `HKCU\<subkey>` 下写入一个字符串值。
///
/// 若子键不存在则自动创建。
#[cfg(windows)]
pub fn set_hkcu_string(subkey: &str, value_name: &str, value: &str) -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(subkey)?;
    key.set_value(value_name, &value.to_string())?;
    Ok(())
}

/// 删除 `HKCU\<subkey>` 下的一个值。
///
/// 若子键或值不存在则静默忽略。
#[cfg(windows)]
pub fn delete_hkcu_value(subkey: &str, value_name: &str) -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(subkey, KEY_ALL_ACCESS) {
        let _ = key.delete_value(value_name);
    }
    Ok(())
}

/// 读取 `HKCU\<subkey>` 下的字符串值。
///
/// 子键或值不存在时返回 `None`。
#[cfg(windows)]
pub fn get_hkcu_string(subkey: &str, value_name: &str) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(subkey)
        .ok()
        .and_then(|k| k.get_value::<String, _>(value_name).ok())
}

// ── 非 Windows 桩 ────────────────────────────────────────────────

#[cfg(not(windows))]
pub fn set_hkcu_string(_subkey: &str, _value_name: &str, _value: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn delete_hkcu_value(_subkey: &str, _value_name: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn get_hkcu_string(_subkey: &str, _value_name: &str) -> Option<String> {
    None
}
