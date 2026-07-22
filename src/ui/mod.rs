//! 前端入口集合。
//!
//! - `tui` — ratatui 终端全屏界面（无参数启动时进入）
//! - `gui` — egui/eframe 轻量即时模式 GUI（通过 `[[bin]] MiPCM_GUI` 入口编译）

pub mod tui;
