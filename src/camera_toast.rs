//! 任务 2：AutoCloseCameraToast —— 从源头精准抑制「请确认摄像头状态」弹窗。
//!
//! 经反编译与资源解析确认：该弹窗（资源键 `CameraCheckStateTitle`）只在
//! `PcControlCenter.Services.UI.MainView.Instances.SynergyUIService` 的相机异常回调
//! `ICameraCooperationWrapperUI.ExceptionCallback(CameraExceptionId, string)` 收到
//! `kLOCAL_CAMERA_DISABLED`（枚举值 3）时弹出。
//!
//! 因此补丁在该方法体最前面注入 `if (exception_id == 3) return;`：
//! 只屏蔽这一个误报 nag，保留权限提示、摄像头被占用、连接断开等其它有用提示；
//! 蓝牙、来电、耳机、通用 Toast 及虚拟摄像头的设备状态逻辑完全不受影响。

use crate::dotnet::{InjectOutcome, inject_guard_return, pe::PeImage};
use crate::install;
use anyhow::Result;
use std::path::Path;

/// 目标程序集文件名。
pub const TARGET_DLL: &str = "PcControlCenter.dll";

/// 目标类型简单名。
const TYPE_NAME: &str = "SynergyUIService";
/// 目标方法名后缀（显式接口实现，元数据中带接口全名前缀）。
const METHOD_SUFFIX: &str = "ExceptionCallback";
/// 实例方法参数下标：arg0=this，arg1=exception_id。
const ARG_INDEX: u8 = 1;
/// `CameraExceptionId.kLOCAL_CAMERA_DISABLED` 的枚举值。
const KLOCAL_CAMERA_DISABLED: i32 = 3;
/// 注入方法体所在新节名（≤8 字节）。
const SECTION_NAME: &str = ".mipatch";

/// 对 `PcControlCenter.dll` 应用补丁。
pub fn apply(dll_path: &Path) -> Result<InjectOutcome> {
    install::ensure_backup(dll_path)?;
    let data = std::fs::read(dll_path)?;
    let mut pe = PeImage::parse(data)?;
    let outcome = inject_guard_return(
        &mut pe,
        TYPE_NAME,
        METHOD_SUFFIX,
        ARG_INDEX,
        KLOCAL_CAMERA_DISABLED,
        SECTION_NAME,
    )?;
    if outcome == InjectOutcome::Patched {
        install::write_file_atomic(dll_path, &pe.data)?;
    }
    Ok(outcome)
}

/// 从备份还原 `PcControlCenter.dll`。
pub fn revert(dll_path: &Path) -> Result<()> {
    install::restore_backup(dll_path)
}
