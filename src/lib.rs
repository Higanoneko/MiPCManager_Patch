//! 小米电脑管家 / 小米互联 补丁工具核心库。
//!
//! CLI（`src/main.rs`）与轻量 GUI（`src/bin/gui.rs`）共享这里的模块与 [`ops`] 高层操作，
//! 确保两个前端调用完全相同的逻辑：状态查看、地区伪装、摄像头弹窗、音频流转、设备伪装、安装。

pub mod camera_toast;
pub mod device_spoof;
pub mod dotnet;
pub mod elevate;
pub mod install;
pub mod locale_spoof;
pub mod mipcaudio_lan;
pub mod ops;
pub mod pc_manager_installer;
