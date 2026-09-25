//! 小米电脑管家 / 小米互联 / 超级小爱补丁工具核心库。
//!
//! CLI（`src/main.rs`）与 GUI（`src/ui/gui.rs`）共享这里的模块与 [`ops`] 高层操作，
//! 确保两个前端调用完全相同的逻辑：状态查看、地区伪装、摄像头弹窗、鼠标快捷键、音频流转、设备伪装、超级小爱补丁、安装。

pub mod i18n;
pub mod infra;
pub mod patches;

pub mod elevate;
pub mod ops;

pub mod experimental;
pub mod install;
pub mod uninstall;

#[cfg(feature = "cli")]
pub mod ui;
