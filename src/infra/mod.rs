//! 通用基础设施模块（infra）。
//!
//! 集中提供项目中被多处重复使用的基础能力：
//! - [`pe`]：PE32+ 文件解析与变换
//! - [`bytes`]：字节序列搜索与单字节定位
//! - [`registry`]：Windows 注册表读写（`#[cfg(windows)]`）

pub mod bytes;
pub mod pe;
pub mod powershell;
pub mod registry;
