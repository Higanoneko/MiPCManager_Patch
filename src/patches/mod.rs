//! 补丁模块集合。
//!
//! 将各项补丁模块统一收纳至此：
//! - [`ai`]：超级小爱安装 / 运行代理（userenv.dll）
//! - [`locale`]：地区伪装（micont_rtm.dll 字节补丁 + 注册表）
//! - [`camera`]：摄像头弹窗抑制（PcControlCenter.dll .NET 方法体注入）
//! - [`audio`]：音频流转广播模式（IfType 补丁 + Wi-Fi 本地子网路由）
//! - [`device`]：设备伪装（代理 DLL 释放 + 注册表机型）
//! - [`xiaomi_share_menu`]：Windows 11 一级右键“小米互传”
//!
//! 内部实现已切换至 [`crate::infra`] 公共工具。

pub mod ai;
pub mod audio;
pub mod camera;
pub mod device;
pub mod locale;
pub mod xiaomi_share_menu;
