//! 快捷键设置页鼠标按键支持。
//!
//! 小米电脑管家的 Web 设置页已经能把中键、右键编码为
//! `MMouseButtonDown` / `RMouseButtonDown`，但没有捕获浏览器侧键；
//! `MiScreenShare.exe` 则已经原生识别 `XMouseButtonDown` 与
//! `XMouseButtonDown2`。本补丁补齐设置页的侧键映射，并允许“打开搜索”
//! 使用原本被界面禁用的中键和右键。

use crate::{infra, install};
use anyhow::{Context, Result, bail};
use std::path::Path;

/// 相对于小米电脑管家版本目录的目标文件。
pub const TARGET_RELATIVE_PATH: &str = "dist/static/js/main.js";

const ORIGINAL_MOUSE_HANDLER: &[u8] = br#"onMouseDown:e=>{1!==e.button||d||(e.preventDefault(),j("MMouseButtonDown")),2!==e.button||u||(e.preventDefault(),j("RMouseButtonDown"))}"#;
const PATCHED_MOUSE_HANDLER: &[u8] = br#"onMouseDown:e=>{1!==e.button||d||(e.preventDefault(),j("MMouseButtonDown")),2!==e.button||u||(e.preventDefault(),j("RMouseButtonDown")),3!==e.button||(e.preventDefault(),j("XMouseButtonDown")),4!==e.button||(e.preventDefault(),j("XMouseButtonDown2"))}"#;

const ORIGINAL_SEARCH_FLAGS: &[u8] = b"allowThree:!0,disableRightKey:!0,disableMiddleKey:!0";
const PATCHED_SEARCH_FLAGS: &[u8] = b"allowThree:!0,disableRightKey:!1,disableMiddleKey:!1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    Patched,
    AlreadyPatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    Original,
    Patched,
    Partial,
    Unknown,
}

impl PatchState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "未应用",
            Self::Patched => "已启用鼠标按键",
            Self::Partial => "部分补丁（建议还原后重试）",
            Self::Unknown => "无法识别当前版本",
        }
    }
}

/// 修改压缩后的 Web 设置页脚本。所有特征均要求唯一，防止版本变化时误改。
pub fn patch_bytes(data: &mut Vec<u8>) -> Result<PatchOutcome> {
    let mouse_changed = replace_signature(
        data,
        ORIGINAL_MOUSE_HANDLER,
        PATCHED_MOUSE_HANDLER,
        "快捷键鼠标事件处理器",
    )?;
    let search_changed = replace_signature(
        data,
        ORIGINAL_SEARCH_FLAGS,
        PATCHED_SEARCH_FLAGS,
        "搜索快捷键鼠标按键开关",
    )?;

    if mouse_changed || search_changed {
        Ok(PatchOutcome::Patched)
    } else {
        Ok(PatchOutcome::AlreadyPatched)
    }
}

pub fn state_bytes(data: &[u8]) -> PatchState {
    let original_mouse = infra::bytes::find_bytes(data, ORIGINAL_MOUSE_HANDLER).is_some();
    let patched_mouse = infra::bytes::find_bytes(data, PATCHED_MOUSE_HANDLER).is_some();
    let original_search = infra::bytes::find_bytes(data, ORIGINAL_SEARCH_FLAGS).is_some();
    let patched_search = infra::bytes::find_bytes(data, PATCHED_SEARCH_FLAGS).is_some();

    match (
        original_mouse,
        patched_mouse,
        original_search,
        patched_search,
    ) {
        (true, false, true, false) => PatchState::Original,
        (false, true, false, true) => PatchState::Patched,
        (false, false, false, false) => PatchState::Unknown,
        _ => PatchState::Partial,
    }
}

pub fn current_state(path: &Path) -> Result<PatchState> {
    let data = std::fs::read(path)
        .with_context(|| format!("无法读取快捷键设置脚本 {}", path.display()))?;
    Ok(state_bytes(&data))
}

pub fn apply(path: &Path) -> Result<PatchOutcome> {
    install::ensure_backup(path)?;
    let mut data = std::fs::read(path)
        .with_context(|| format!("无法读取快捷键设置脚本 {}", path.display()))?;
    let outcome = patch_bytes(&mut data)?;
    if outcome == PatchOutcome::Patched {
        install::write_file_atomic(path, &data)?;
    }
    Ok(outcome)
}

pub fn revert(path: &Path) -> Result<()> {
    install::restore_backup(path)
}

fn replace_signature(
    data: &mut Vec<u8>,
    original: &[u8],
    patched: &[u8],
    label: &str,
) -> Result<bool> {
    let original_position = infra::bytes::find_bytes(data, original);
    let patched_position = infra::bytes::find_bytes(data, patched);
    match (original_position, patched_position) {
        (None, Some(position)) => {
            ensure_unique(data, patched, position, label)?;
            Ok(false)
        }
        (Some(position), None) => {
            ensure_unique(data, original, position, label)?;
            data.splice(position..position + original.len(), patched.iter().copied());
            Ok(true)
        }
        (Some(_), Some(_)) => bail!("main.js 同时包含{label}的原始和补丁特征，已中止以免误改"),
        (None, None) => bail!("未在 main.js 中找到{label}特征，可能版本结构已变更"),
    }
}

fn ensure_unique(data: &[u8], signature: &[u8], position: usize, label: &str) -> Result<()> {
    let next = position + 1;
    if next < data.len() && infra::bytes::find_bytes(&data[next..], signature).is_some() {
        bail!("main.js 中的{label}特征不唯一，已中止以免误改");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        [
            b"prefix,".as_slice(),
            ORIGINAL_MOUSE_HANDLER,
            b",middle,".as_slice(),
            ORIGINAL_SEARCH_FLAGS,
            b",suffix".as_slice(),
        ]
        .concat()
    }

    #[test]
    fn patches_mouse_buttons_and_is_idempotent() {
        let mut data = fixture();

        assert_eq!(state_bytes(&data), PatchState::Original);
        assert_eq!(patch_bytes(&mut data).unwrap(), PatchOutcome::Patched);
        assert_eq!(state_bytes(&data), PatchState::Patched);
        assert!(infra::bytes::find_bytes(&data, b"XMouseButtonDown").is_some());
        assert!(infra::bytes::find_bytes(&data, b"XMouseButtonDown2").is_some());
        assert_eq!(
            patch_bytes(&mut data).unwrap(),
            PatchOutcome::AlreadyPatched
        );
    }

    #[test]
    fn rejects_unknown_version() {
        let mut data = b"unrelated javascript".to_vec();
        assert!(patch_bytes(&mut data).is_err());
        assert_eq!(state_bytes(&data), PatchState::Unknown);
    }

    #[test]
    fn completes_a_partial_patch() {
        let mut data = [
            PATCHED_MOUSE_HANDLER,
            b",".as_slice(),
            ORIGINAL_SEARCH_FLAGS,
        ]
        .concat();

        assert_eq!(state_bytes(&data), PatchState::Partial);
        assert_eq!(patch_bytes(&mut data).unwrap(), PatchOutcome::Patched);
        assert_eq!(state_bytes(&data), PatchState::Patched);
    }

    /// 用未入库的小米电脑管家 Web 资源验证真实版本。
    #[test]
    #[ignore = "requires MIPCM_HOTKEY_FIXTURE pointing to the real dist/static/js/main.js"]
    fn patches_real_xiaomi_pc_manager_fixture() {
        let path = std::env::var_os("MIPCM_HOTKEY_FIXTURE")
            .map(std::path::PathBuf::from)
            .expect("MIPCM_HOTKEY_FIXTURE must point to dist/static/js/main.js");
        let original = std::fs::read(path).expect("failed to read XiaomiPCManager main.js");
        let mut patched = original.clone();

        assert_eq!(state_bytes(&original), PatchState::Original);
        assert_eq!(patch_bytes(&mut patched).unwrap(), PatchOutcome::Patched);
        assert_eq!(state_bytes(&patched), PatchState::Patched);
        assert!(patched.len() > original.len());
        assert_eq!(
            patch_bytes(&mut patched).unwrap(),
            PatchOutcome::AlreadyPatched
        );
    }
}
