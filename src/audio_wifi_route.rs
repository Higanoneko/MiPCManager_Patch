//! 为「有线广播 + Wi-Fi 同时在线」场景补足媒体会话的出接口选择。
//!
//! MiPCAudio / idmruntime 中的 IfType 补丁只影响设备发现身份（MAC），真正建立
//! WFD 音频 TCP 会话的是 MiPlayCastService 子进程。若有线和 Wi-Fi 位于同一 IPv4
//! 子网，Windows 会因有线跃点更低而让该会话从有线网卡出站；手机会在收到 PLAY
//! 后立即 TEARDOWN。这里在 Wi-Fi 的本地 IPv4 子网上添加一条更低跃点的持久路由：
//! 它仅覆盖局域网对端，默认路由仍可继续走有线。

use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;

const STATE_FILE: &str = ".mipcm_audio_wifi_route";
const ROUTE_METRIC: u32 = 1;

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
pub fn apply(version_dir: &Path) -> Result<Option<bool>> {
    let Some(subnet) = discover_wifi_subnet()? else {
        return Ok(None);
    };
    let state_path = state_path(version_dir);

    if let Some(existing) = read_state(&state_path)? {
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
    write_state(
        &state_path,
        &RouteState {
            interface_index: subnet.interface_index,
            prefix: subnet.prefix,
        },
    )?;
    Ok(Some(true))
}

/// 仅删除本工具此前创建并记录的路由，不会触碰用户已有路由。
pub fn revert(version_dir: &Path) -> Result<bool> {
    let path = state_path(version_dir);
    let Some(state) = read_state(&path)? else {
        return Ok(false);
    };
    remove_route(&state)?;
    remove_state_file(&path)?;
    Ok(true)
}

pub fn state(version_dir: &Path) -> String {
    match read_state(&state_path(version_dir)) {
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
        ROUTE_METRIC,
        route.prefix,
        route.interface_index,
        ROUTE_METRIC
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
        route.prefix, route.interface_index, ROUTE_METRIC
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

fn state_path(version_dir: &Path) -> PathBuf {
    version_dir.join(STATE_FILE)
}

fn write_state(path: &Path, state: &RouteState) -> Result<()> {
    let data = format!(
        "version=1\ninterface_index={}\nprefix={}\n",
        state.interface_index, state.prefix
    );
    crate::install::write_file_atomic(path, data.as_bytes())
}

fn read_state(path: &Path) -> Result<Option<RouteState>> {
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
