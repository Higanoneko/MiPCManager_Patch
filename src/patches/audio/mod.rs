//! 音频流转补丁：统一 IfType 广播身份 + Wi-Fi 本地子网路由修复。
//!
//! MiPCAudio / idmruntime 中的 IfType 补丁控制设备发现时的广播身份（无线 MAC / 有线 MAC）。
//! 但在无线广播且有线+Wi-Fi 位于同一 IPv4 子网时，Windows 会因有线跃点更低而将
//! MiPlayCastService 的 WFD 音频 TCP 会话从有线网卡出站；手机看到 PLAY 请求的
//! 来源 IP 与发现身份不一致，会立即 TEARDOWN。此时需在 Wi-Fi 子网上添加 metric=1
//! 的持久路由，强制媒体会话走 Wi-Fi。

use crate::{infra, install};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── IfType 补丁（原 mipcaudio_lan）──────────────────────────────

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

/// 一处"选 WiFi 网卡"判定的指令块特征：
/// 匹配 `prefix` ++ (值字节∈{0x47,0x06}) ++ `suffix`，待改写的就是中间那个值字节。
struct Site {
    file: &'static str,
    prefix: &'static [u8],
    suffix: &'static [u8],
}

/// 三处 IfType 选择器。`suffix` 取其后的条件跳转，确保特征唯一。
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

/// 对单个文件应用模式（处理它名下的所有站点）。返回是否有实际改动。
fn patch_file(path: &Path, sites: &[&Site], mode: BroadcastMode) -> Result<FileOutcome> {
    install::ensure_backup(path)?;
    let mut data = std::fs::read(path)?;
    let target = mode.iftype();
    let mut outcome = FileOutcome::default();
    for site in sites {
        let vi = infra::bytes::locate_single_byte(
            &data,
            site.prefix,
            site.suffix,
            &[IFTYPE_WIFI, IFTYPE_ETHERNET],
        )?;
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
            Ok(data) => {
                match infra::bytes::locate_single_byte(
                    &data,
                    s.prefix,
                    s.suffix,
                    &[IFTYPE_WIFI, IFTYPE_ETHERNET],
                ) {
                    Ok(vi) => match data[vi] {
                        IFTYPE_WIFI => "无线(WiFi)".to_string(),
                        IFTYPE_ETHERNET => "有线(LAN)".to_string(),
                        other => format!("未知(0x{other:02X})"),
                    },
                    Err(e) => format!("未定位({e})"),
                }
            }
            Err(_) => "文件不可读".to_string(),
        };
        out.push((s.file.to_string(), state));
    }
    out
}

// ── Wi-Fi 本地子网路由（原 audio_wifi_route）─────────────────────

const WIFI_ROUTE_STATE_FILE: &str = ".mipcm_audio_wifi_route";
const WIFI_ROUTE_METRIC: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WifiSubnet {
    interface_index: u32,
    prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteState {
    interface_index: u32,
    prefix: String,
}

/// 应用由本工具管理的 Wi-Fi 本地子网路由。
///
/// 返回值含义：
/// - `Some(true)`：本次新建路由；
/// - `Some(false)`：此前已有相同路由；
/// - `None`：当前没有可用于媒体会话的 Wi-Fi IPv4 接口，因此未更改网络配置。
pub fn apply_wifi_route(version_dir: &Path) -> Result<Option<bool>> {
    let Some(subnet) = discover_wifi_subnet()? else {
        return Ok(None);
    };
    let state_path = wifi_route_state_path(version_dir);

    if let Some(existing) = read_wifi_route_state(&state_path)? {
        if existing.interface_index == subnet.interface_index && existing.prefix == subnet.prefix {
            return Ok(Some(false));
        }
        remove_route(&existing)?;
        remove_state_file(&state_path)?;
    }

    if !add_route(&subnet)? {
        // 同一条路由可能是用户或先前版本手工创建的；没有状态文件时不接管它，
        // 这样 `revert` 不会误删用户的网络配置。
        return Ok(Some(false));
    }
    write_wifi_route_state(
        &state_path,
        &RouteState {
            interface_index: subnet.interface_index,
            prefix: subnet.prefix,
        },
    )?;
    Ok(Some(true))
}

/// 仅删除本工具此前创建并记录的路由，不会触碰用户已有路由。
pub fn revert_wifi_route(version_dir: &Path) -> Result<bool> {
    let path = wifi_route_state_path(version_dir);
    let Some(state) = read_wifi_route_state(&path)? else {
        return Ok(false);
    };
    remove_route(&state)?;
    remove_state_file(&path)?;
    Ok(true)
}

/// 返回 Wi-Fi 路由状态的展示文本。
pub fn wifi_route_state(version_dir: &Path) -> String {
    match read_wifi_route_state(&wifi_route_state_path(version_dir)) {
        Ok(Some(route)) => format!(
            "已固定 Wi-Fi 本地路由（{}，接口 {}）",
            route.prefix, route.interface_index
        ),
        Ok(None) => "未配置".to_string(),
        Err(error) => format!("状态不可读（{error}）"),
    }
}

fn discover_wifi_subnet() -> Result<Option<WifiSubnet>> {
    // `InterfaceType` 是 IANA IfType：71 = IEEE 802.11。使用数值而非本地化名称。
    // 仅选择物理、已连接且拥有可用 IPv4 地址的无线接口。
    let script = r#"
$wifi = Get-NetAdapter -Physical -ErrorAction Stop |
    Where-Object {
        $interfaceType = if ($null -ne $_.InterfaceType) { [int]$_.InterfaceType } else { [int]$_.ifType }
        $_.Status -eq 'Up' -and $interfaceType -eq 71
    } |
    ForEach-Object {
        $adapter = $_
        $ip = Get-NetIPAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object {
                $_.IPAddress -ne '127.0.0.1' -and
                -not $_.IPAddress.StartsWith('169.254.') -and
                $_.PrefixLength -gt 0 -and $_.PrefixLength -lt 32 -and
                $_.AddressState -eq 'Preferred'
            } |
            Select-Object -First 1
        if ($null -ne $ip) {
            $metric = (Get-NetIPInterface -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction Stop).InterfaceMetric
            [PSCustomObject]@{ Index = [int]$adapter.ifIndex; Address = $ip.IPAddress; Prefix = [int]$ip.PrefixLength; Metric = [int]$metric }
        }
    } |
    Sort-Object Metric, Index |
    Select-Object -First 1
if ($null -ne $wifi) { '{0}|{1}|{2}' -f $wifi.Index, $wifi.Address, $wifi.Prefix }
"#;
    let output = run_powershell(script).context("查询 Wi-Fi IPv4 接口失败")?;
    let Some(line) = output.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return Ok(None);
    };
    parse_wifi_subnet(line).map(Some)
}

fn add_route(route: &WifiSubnet) -> Result<bool> {
    let script = format!(
        "$existing = Get-NetRoute -PolicyStore PersistentStore -DestinationPrefix '{}' -InterfaceIndex {} -NextHop '0.0.0.0' -ErrorAction SilentlyContinue | Where-Object {{ $_.RouteMetric -eq {} }} | Select-Object -First 1; if ($null -ne $existing) {{ 'existing' }} else {{ New-NetRoute -PolicyStore PersistentStore -DestinationPrefix '{}' -InterfaceIndex {} -NextHop '0.0.0.0' -RouteMetric {} -ErrorAction Stop | Out-Null; 'created' }}",
        route.prefix,
        route.interface_index,
        WIFI_ROUTE_METRIC,
        route.prefix,
        route.interface_index,
        WIFI_ROUTE_METRIC
    );
    let output = run_powershell(&script)
        .with_context(|| format!("添加 Wi-Fi 本地路由 {} 失败", route.prefix))?;
    match output.trim() {
        "created" => Ok(true),
        "existing" => Ok(false),
        other => bail!("添加 Wi-Fi 本地路由未返回预期结果：{other}"),
    }
}

fn remove_route(route: &RouteState) -> Result<()> {
    let script = format!(
        "Get-NetRoute -PolicyStore PersistentStore -DestinationPrefix '{}' -InterfaceIndex {} -NextHop '0.0.0.0' -ErrorAction SilentlyContinue | Where-Object {{ $_.RouteMetric -eq {} }} | Remove-NetRoute -PolicyStore PersistentStore -Confirm:$false -ErrorAction Stop",
        route.prefix, route.interface_index, WIFI_ROUTE_METRIC
    );
    run_powershell(&script)
        .with_context(|| format!("删除 Wi-Fi 本地路由 {} 失败", route.prefix))?;
    Ok(())
}

fn run_powershell(script: &str) -> Result<String> {
    #[cfg(not(windows))]
    {
        let _ = script;
        bail!("Wi-Fi 本地路由功能仅支持 Windows");
    }
    #[cfg(windows)]
    {
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .context("无法启动 Windows PowerShell")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            bail!("PowerShell 退出码 {:?}：{}", output.status.code(), stderr);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

fn wifi_route_state_path(version_dir: &Path) -> PathBuf {
    version_dir.join(WIFI_ROUTE_STATE_FILE)
}

fn write_wifi_route_state(path: &Path, state: &RouteState) -> Result<()> {
    let data = format!(
        "version=1\ninterface_index={}\nprefix={}\n",
        state.interface_index, state.prefix
    );
    install::write_file_atomic(path, data.as_bytes())
}

fn read_wifi_route_state(path: &Path) -> Result<Option<RouteState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("读取路由状态文件失败 {}", path.display()))?;
    let mut interface_index = None;
    let mut prefix = None;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("interface_index=") {
            interface_index = Some(value.parse::<u32>().context("路由状态中的接口索引无效")?);
        } else if let Some(value) = line.strip_prefix("prefix=") {
            prefix = Some(validate_prefix(value)?.to_string());
        }
    }
    match (interface_index, prefix) {
        (Some(interface_index), Some(prefix)) => Ok(Some(RouteState {
            interface_index,
            prefix,
        })),
        _ => bail!("路由状态文件格式无效 {}", path.display()),
    }
}

fn remove_state_file(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("删除路由状态文件失败 {}", path.display()))?;
    }
    Ok(())
}

fn parse_wifi_subnet(line: &str) -> Result<WifiSubnet> {
    let mut fields = line.split('|');
    let interface_index = fields
        .next()
        .ok_or_else(|| anyhow!("Wi-Fi 接口查询结果缺少索引"))?
        .parse::<u32>()
        .context("Wi-Fi 接口索引无效")?;
    let address = fields
        .next()
        .ok_or_else(|| anyhow!("Wi-Fi 接口查询结果缺少 IPv4 地址"))?
        .parse::<Ipv4Addr>()
        .context("Wi-Fi IPv4 地址无效")?;
    let prefix_length = fields
        .next()
        .ok_or_else(|| anyhow!("Wi-Fi 接口查询结果缺少前缀长度"))?
        .parse::<u8>()
        .context("Wi-Fi IPv4 前缀长度无效")?;
    if fields.next().is_some() {
        bail!("Wi-Fi 接口查询结果格式无效");
    }
    Ok(WifiSubnet {
        interface_index,
        prefix: ipv4_prefix(address, prefix_length)?,
    })
}

fn ipv4_prefix(address: Ipv4Addr, prefix_length: u8) -> Result<String> {
    if prefix_length == 0 || prefix_length >= 32 {
        bail!("Wi-Fi IPv4 前缀长度必须在 1 到 31 之间");
    }
    let mask = u32::MAX << (32 - prefix_length);
    let network = u32::from(address) & mask;
    Ok(format!("{}/{}", Ipv4Addr::from(network), prefix_length))
}

fn validate_prefix(value: &str) -> Result<&str> {
    let (address, prefix_length) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("IPv4 路由前缀无效"))?;
    let address = address.parse::<Ipv4Addr>().context("IPv4 路由地址无效")?;
    let prefix_length = prefix_length
        .parse::<u8>()
        .context("IPv4 路由前缀长度无效")?;
    let normalized = ipv4_prefix(address, prefix_length)?;
    if normalized != value {
        bail!("IPv4 路由前缀必须是网络地址");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_local_wifi_prefix_without_changing_default_route() {
        assert_eq!(
            ipv4_prefix("192.168.50.123".parse().unwrap(), 24).unwrap(),
            "192.168.50.0/24"
        );
        assert_eq!(
            ipv4_prefix("10.33.42.7".parse().unwrap(), 16).unwrap(),
            "10.33.0.0/16"
        );
    }

    #[test]
    fn parses_the_adapter_data_emitted_by_powershell() {
        assert_eq!(
            parse_wifi_subnet("13|192.168.50.123|24").unwrap(),
            WifiSubnet {
                interface_index: 13,
                prefix: "192.168.50.0/24".to_string(),
            }
        );
    }

    #[test]
    fn rejects_non_network_or_default_prefixes() {
        assert!(validate_prefix("192.168.50.123/24").is_err());
        assert!(ipv4_prefix("192.168.50.123".parse().unwrap(), 0).is_err());
        assert!(ipv4_prefix("192.168.50.123".parse().unwrap(), 32).is_err());
    }
}
