//! 实验性双网卡同网段音频修复。
//!
//! 当有线 + Wi-Fi 两个网卡同时在线且位于同一 IPv4 子网时，Windows 优先使用跃点
//! （InterfaceMetric）更低的网卡作为出站接口。这导致：
//!
//! - 无线广播模式下：发现身份走 Wi-Fi，但媒体 TCP 会话从有线出站 → 不一致 → TEARDOWN
//!
//! 本模块提供一套自检 + 自动修复机制：
//! 1. 检测当前是否处于双网卡同网段状态
//! 2. 根据当前 IfType 广播模式，判断 Wi-Fi 本地子网路由是否匹配
//! 3. 不匹配时自动创建或移除路由，使发现身份与媒体出站统一

use crate::patches::audio;
use anyhow::Result;
use std::path::Path;

/// 双网卡诊断结果。
#[derive(Debug)]
pub struct DualNicDiagnosis {
    /// 有线网卡数量
    pub wired_adapters: usize,
    /// Wi-Fi 网卡数量
    pub wifi_adapters: usize,
    /// 是否有同网段冲突（至少一张有线 + 一张 Wi-Fi 位于同一子网）
    pub same_subnet_conflict: bool,
    /// 当前 IfType 广播模式（来自已打补丁的文件）
    pub broadcast_mode: Option<String>,
    /// Wi-Fi 路由是否已就位
    pub wifi_route_active: bool,
    /// 当前配置是否一致（发现身份与媒体出站走同一张网卡）
    pub consistent: bool,
}

/// 诊断当前双网卡状态，返回可展示的诊断信息行。
pub fn diagnose(version_dir: &Path) -> Result<Vec<String>> {
    let mut log = Vec::new();
    log.push("== 双网卡音频诊断 ==".to_string());

    let states = audio::current_state(version_dir);
    if states.is_empty() {
        log.push("  未找到 MiPCAudio.exe / idmruntime.dll，请先确认版本目录。".to_string());
        return Ok(log);
    }

    let first_state = &states[0].1;
    let mode_is_wifi = first_state.contains("无线") || first_state.contains("WiFi");
    let mode_is_lan = first_state.contains("有线") || first_state.contains("LAN");
    log.push(format!("  当前广播模式: {first_state}"));

    let route_state = audio::wifi_route_state(version_dir);
    let route_active = !route_state.contains("未配置");
    log.push(format!("  Wi-Fi 优先路由: {}", route_state));

    let consistent = match (mode_is_wifi, mode_is_lan, route_active) {
        (true, _, true) | (_, true, false) => {
            log.push("  ✓ 发现身份与媒体出站一致。".to_string());
            true
        }
        (true, _, false) => {
            log.push(
                "  ✗ 无线广播模式但 Wi-Fi 路由缺失 —— 同网段时媒体可能从有线出站，手机将拒绝。"
                    .to_string(),
            );
            false
        }
        (_, true, true) => {
            log.push(
                "  ✗ 有线广播模式但 Wi-Fi 路由仍存在 —— 发现走有线、媒体走 Wi-Fi，手机将拒绝。"
                    .to_string(),
            );
            false
        }
        _ => {
            log.push("  ? 无法判断广播模式，请手动检查。".to_string());
            false
        }
    };

    log.push(format!(
        "  建议: {}",
        if consistent {
            "当前配置正确，无需操作。"
        } else if mode_is_wifi {
            "运行「实验性修复」以添加 Wi-Fi 优先路由。"
        } else {
            "运行「实验性修复」以移除 Wi-Fi 路由。"
        }
    ));

    Ok(log)
}

/// 自动修复：根据当前 IfType 状态，确保 Wi-Fi 路由与之匹配。
///
/// - 无线广播 → 添加 Wi-Fi 路由（若缺失）
/// - 有线广播 → 移除 Wi-Fi 路由（若存在）
///
/// 返回操作日志。
pub fn auto_fix(version_dir: &Path) -> Result<Vec<String>> {
    let mut log = Vec::new();
    log.push("== 双网卡音频实验性修复 ==".to_string());

    let states = audio::current_state(version_dir);
    if states.is_empty() {
        log.push("  未找到 MiPCAudio.exe / idmruntime.dll。".to_string());
        return Ok(log);
    }

    let first_state = &states[0].1;
    let mode_is_wifi = first_state.contains("无线") || first_state.contains("WiFi");
    log.push(format!("  当前广播模式: {first_state}"));

    if mode_is_wifi {
        match audio::apply_wifi_route(version_dir)? {
            Some(true) => log.push(
                "  ✓ 已添加 Wi-Fi 本地子网优先路由（metric=1）：媒体会话将走 Wi-Fi。".to_string(),
            ),
            Some(false) => log.push("  • Wi-Fi 本地子网优先路由已存在，无需操作。".to_string()),
            None => log.push(
                "  ✗ 未检测到可用 Wi-Fi IPv4 接口，无法添加路由。请确认 Wi-Fi 已连接。".to_string(),
            ),
        }
    } else {
        let removed = audio::revert_wifi_route(version_dir)?;
        if removed {
            log.push(
                "  ✓ 已移除 Wi-Fi 本地子网优先路由：有线模式下发现与媒体均走有线。".to_string(),
            );
        } else {
            log.push("  • 不存在本工具管理的 Wi-Fi 路由，无需操作。".to_string());
        }
    }

    log.push("  提示：请手动重新启动小米电脑管家使修复生效。".to_string());
    Ok(log)
}
