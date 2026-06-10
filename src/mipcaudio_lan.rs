//! 任务 3：MiPCAudio 音频流转的「无线 / 有线」广播模式统一切换。
//!
//! 背景：设备身份（用作发现去重的 MAC）由「选取某张网卡的 MAC」决定。经反汇编确认，
//! 选网卡的判定 `IfType == IF_TYPE_IEEE80211(0x47, WiFi)` 一共出现在 **三处**：
//!   - `MiPCAudio.exe`（Lyra/netbus 的 `GetMacIp`，type8 身份）
//!   - `idmruntime.dll`（IDM 的「get WiFi Adapter MAC」，type2 身份）
//!   - `idmruntime.dll`（IDM 另一条取 MAC 路径）
//!
//! 设备同时通过 Lyra(type8) 与 IDM(type2) 两套机制发布。只要三处身份一致，手机就合并为
//! 单设备；若只改其中一处（如旧版 txt 补丁），身份分歧 → 出现重复设备。
//!
//! 由于「自动跟随当前活跃出口网卡」需要遍历比较 `Ipv4Metric`，无法用等长字节替换实现，
//! 这里采用更简单可靠的方案：**统一**把三处改成同一介质，交由用户按自己的接入方式选择：
//!   - 无线模式：三处 = `0x47`（WiFi，等同原始出厂状态）
//!   - 有线模式：三处 = `0x06`（IF_TYPE_ETHERNET_CSMACD，走有线 LAN）
//!
//! 定位采用「指令块特征（prefix + 值字节 + 后随 jne）」而非硬编码偏移，跨版本稳定。

use crate::install;
use anyhow::{Result, anyhow, bail};
use std::collections::BTreeMap;
use std::path::Path;

pub const TARGET_MIPCAUDIO: &str = "MiPCAudio.exe";
pub const TARGET_IDMRUNTIME: &str = "idmruntime.dll";

/// IF_TYPE_IEEE80211：无线。
const IFTYPE_WIFI: u8 = 0x47;
/// IF_TYPE_ETHERNET_CSMACD：有线。
const IFTYPE_ETHERNET: u8 = 0x06;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BroadcastMode {
    Wireless,
    Wired,
}

impl BroadcastMode {
    fn iftype(self) -> u8 {
        match self {
            BroadcastMode::Wireless => IFTYPE_WIFI,
            BroadcastMode::Wired => IFTYPE_ETHERNET,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            BroadcastMode::Wireless => "无线 (WiFi)",
            BroadcastMode::Wired => "有线 (LAN)",
        }
    }
}

/// 一处「选 WiFi 网卡」判定的指令块特征：
/// 匹配 `prefix` ++ (值字节∈{0x47,0x06}) ++ `suffix`，待改写的就是中间那个值字节。
struct Site {
    file: &'static str,
    prefix: &'static [u8],
    suffix: &'static [u8],
}

/// 三处 IfType 选择器（详见模块文档）。`suffix` 取其后的条件跳转，确保特征唯一。
const SITES: &[Site] = &[
    // MiPCAudio.exe: cmp dword [r9+0x64], 0x47 ; jne (short)
    Site {
        file: TARGET_MIPCAUDIO,
        prefix: &[0x41, 0x83, 0x79, 0x64],
        suffix: &[0x75],
    },
    // idmruntime.dll: cmp dword [r14+0x64], 0x47 ; jne (near)
    Site {
        file: TARGET_IDMRUNTIME,
        prefix: &[0x41, 0x83, 0x7e, 0x64],
        suffix: &[0x0F, 0x85],
    },
    // idmruntime.dll: cmp dword [rbx+0x64], 0x47 ; jne (short)
    Site {
        file: TARGET_IDMRUNTIME,
        prefix: &[0x83, 0x7b, 0x64],
        suffix: &[0x75],
    },
];

/// 单个文件的处理结果。
#[derive(Default)]
pub struct FileOutcome {
    pub patched: usize,
    pub already: usize,
}

/// 在 `data` 中按特征唯一定位「值字节」的下标。0 处或多处均视为错误。
fn locate_value_byte(data: &[u8], prefix: &[u8], suffix: &[u8]) -> Result<usize> {
    let span = prefix.len() + 1 + suffix.len();
    if data.len() < span {
        bail!("文件过小");
    }
    let mut found: Option<usize> = None;
    for i in 0..=data.len() - span {
        if &data[i..i + prefix.len()] != prefix {
            continue;
        }
        let vi = i + prefix.len();
        let v = data[vi];
        if (v == IFTYPE_WIFI || v == IFTYPE_ETHERNET)
            && &data[vi + 1..vi + 1 + suffix.len()] == suffix
        {
            if found.is_some() {
                bail!("特征不唯一，疑似版本结构变更，已中止以免误改");
            }
            found = Some(vi);
        }
    }
    found.ok_or_else(|| anyhow!("未找到 IfType 选择器特征，可能版本结构已变更"))
}

/// 对单个文件应用模式（处理它名下的所有站点）。返回是否有实际改动。
fn patch_file(path: &Path, sites: &[&Site], mode: BroadcastMode) -> Result<FileOutcome> {
    install::ensure_backup(path)?;
    let mut data = std::fs::read(path)?;
    let target = mode.iftype();
    let mut outcome = FileOutcome::default();
    for site in sites {
        let vi = locate_value_byte(&data, site.prefix, site.suffix)?;
        if data[vi] == target {
            outcome.already += 1;
        } else {
            data[vi] = target;
            outcome.patched += 1;
        }
    }
    if outcome.patched > 0 {
        install::write_file_atomic(path, &data)?;
    }
    Ok(outcome)
}

/// 应用广播模式：把三处选择器统一为所选介质。`version_dir` 为安装的版本目录。
pub fn apply(version_dir: &Path, mode: BroadcastMode) -> Result<Vec<(String, FileOutcome)>> {
    // 按文件分组站点，每个文件只读写一次。
    let mut by_file: BTreeMap<&str, Vec<&Site>> = BTreeMap::new();
    for s in SITES {
        by_file.entry(s.file).or_default().push(s);
    }
    let mut results = Vec::new();
    for (file, sites) in by_file {
        let path = version_dir.join(file);
        if !path.exists() {
            bail!("未找到目标文件：{}", path.display());
        }
        let outcome = patch_file(&path, &sites, mode)?;
        results.push((file.to_string(), outcome));
    }
    Ok(results)
}

/// 还原两个目标文件（从备份恢复出厂字节）。
pub fn revert(version_dir: &Path) -> Result<()> {
    for file in [TARGET_MIPCAUDIO, TARGET_IDMRUNTIME] {
        let path = version_dir.join(file);
        if path.exists() {
            install::restore_backup(&path)?;
        }
    }
    Ok(())
}

/// 读取当前各站点的介质状态，用于 status 展示。
pub fn current_state(version_dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in SITES {
        let path = version_dir.join(s.file);
        let state = match std::fs::read(&path) {
            Ok(data) => match locate_value_byte(&data, s.prefix, s.suffix) {
                Ok(vi) => match data[vi] {
                    IFTYPE_WIFI => "无线(WiFi)".to_string(),
                    IFTYPE_ETHERNET => "有线(LAN)".to_string(),
                    other => format!("未知(0x{other:02X})"),
                },
                Err(e) => format!("未定位({e})"),
            },
            Err(_) => "文件不可读".to_string(),
        };
        out.push((s.file.to_string(), state));
    }
    out
}
