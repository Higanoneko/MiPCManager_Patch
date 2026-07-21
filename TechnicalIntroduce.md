# Technical Introduce

本文档记录 MiPCManager Patcher 的补丁原理、定位方式、实现细节与构建说明。面向普通用户的功能说明见 [README.md](README.md)。

## 总体设计

工具会自动探测 `XiaomiPCManager` 与 `PcContinuity` 的最新安装版本。完整版 `XiaomiPCManager` 保留启动时全量关闭相关进程的行为；`PcContinuity` 不做全量关闭，只在 LocaleSpoof 动作前处理 `micont_service.exe`。各补丁动作前会按功能关闭对应进程，备份原文件后再应用补丁。`PcContinuity` 暂时只具有 LocaleSpoof 能力，其他功能的自动解析和显式路径都会拒绝该安装目录。若 Patch/还原时遇到 access denied，会自动关闭对应进程并重试一次。所有补丁均幂等且可还原。

补丁定位采用「特征锚点 / 类型名+方法名 / 指令块特征」而非硬编码偏移，因此可适配未来版本。修改 `Program Files` 下文件需管理员权限，release exe 内嵌 `requireAdministrator` manifest，双击启动即弹 UAC，运行时仍保留提权兜底。

## 补丁一：LocaleSpoof（地区伪装）

**目标**：`micont_rtm.dll`（原生 PE64）

**原理**：该 DLL 原本读取 `HKCU\Control Panel\International\Geo\Name`（系统真实地区）。对比作者提供的 `Original` 与 `Patched` 发现全文件仅 4 字节差异，集中在一处宽字符串：紧邻 `...International\Geo\0` 之后的值名 `Name` 被改为 `XCN`。

| 文件偏移 | Original (UTF-16LE) | Patched (UTF-16LE) |
|---|---|---|
| `Geo\0` 之后 | `N a m e \0`（`4E00 6100 6D00 6500 0000`） | `X C N \0 \0`（`5800 4300 4E00 0000 0000`） |

于是程序改读同一注册表键下的 `XCN` 值；工具再向该键写入 `XCN=CN`，程序便读到地区 `CN`，而系统真实 `Name` 保持不变。

**实现**：以宽字符串 `Geo\0` 作锚点，把其后 10 字节的 `Name\0\0` 等长替换为 `XCN\0\0`（不移位、不依赖偏移）。本工具输出与作者黄金参考 `micont_rtm.patched.dll` 逐字节一致（已验证）。

代码：[`src/locale_spoof.rs`](src/locale_spoof.rs)

> 实现思路：感谢 Coolapk@Na1veMagic

## 补丁二：AutoCloseCameraToast（精准抑制摄像头弹窗）

**目标**：`PcControlCenter.dll`（.NET / WinUI 托管程序集）

经反编译与资源解析（`makepri dump`）确认：「请确认摄像头状态」弹窗（资源键 `CameraCheckStateTitle`，内容为「摄像头暂不可用，点击确定打开设备管理器…」）只在相机协同服务 `SynergyUIService` 的相机异常回调 `ICameraCooperationWrapperUI.ExceptionCallback(CameraExceptionId exception_id, …)` 收到 `kLOCAL_CAMERA_DISABLED`（枚举值 3，本机摄像头被误判为禁用）时才弹出。

因此本补丁在该方法体最前面注入一段等价于：

```csharp
if (exception_id == CameraExceptionId.kLOCAL_CAMERA_DISABLED)
    return;
```

的 IL 守卫：只屏蔽这一个误报，保留权限提示、摄像头被占用、连接断开等其它有用提示；蓝牙、来电、耳机、通用 Toast 以及虚拟摄像头的设备状态逻辑全部不受影响。

### 为什么用「重定位」而非元数据改写

注入需要的字符串/常量（枚举值 3 是 IL 立即数）无需向元数据堆新增任何条目，所以避开了「纯 Rust 元数据写库无法回写大型 WinRT 程序集」的难题。由于注入使方法体增长、无法原地扩展，工具的做法是：

1. 纯 Rust 解析 ECMA-335 元数据，按「类型名 + 方法名后缀」定位 `MethodDef`，取得方法体 RVA 与 RVA 字段的文件偏移（[`src/dotnet/metadata.rs`](src/dotnet/metadata.rs)）。
2. 解析方法体（fat/tiny 头、EH 段），在 IL 前拼接 5 字节守卫 `ldarg.1; ldc.i4.3; bne.un.s +1; ret`，必要时整体修正 EH 偏移（[`src/dotnet/method_body.rs`](src/dotnet/method_body.rs)）。
3. 追加一个新节 `.mipatch` 写入新方法体，丢弃失效的 Authenticode 证书、维护 `SizeOfImage`、重算 PE 校验和，并把 `MethodDef.RVA` 改指到新节（[`src/dotnet/pe.rs`](src/dotnet/pe.rs)）。

**验证**：补丁后程序集可被 ILSpy 正常反编译，`ExceptionCallback` 反编译结果即为上述守卫；PE 结构、元数据、方法体头部（codesize/maxstack/局部签名）均合法；重复执行幂等。

代码：[`src/camera_toast.rs`](src/camera_toast.rs)、[`src/dotnet/`](src/dotnet/)

## 补丁三：MiPCAudio 音频流转「无线 / 有线」模式

**目标**：`MiPCAudio.exe` + `idmruntime.dll`（均为原生 PE）

**背景**：手机对电脑做音频流转，原本只有电脑用 WiFi 作主连接时可用；想走有线 LAN 时，旧做法（IDA 把 `MiPCAudio.exe` 里 `cmp [r9+0x64], 0x47` 的 `0x47` 改成 `0x06`）虽能启用有线，却会出现重复设备。

**根因（经反汇编 + 运行日志确认）**：电脑同时通过两套机制把自己发布为音频目标：`Lyra/netbus`（type8）与 `IDM`（type2，`idmruntime.dll`）。两套的设备身份都取自「某张网卡的 MAC」，而选网卡的判定 `IfType == IF_TYPE_IEEE80211(0x47, WiFi)` 一共有三处：

| 二进制 | 指令块特征 | 角色 |
|---|---|---|
| `MiPCAudio.exe` | `41 83 79 64 47 75` | Lyra `GetMacIp`（type8 身份） |
| `idmruntime.dll` | `41 83 7e 64 47 0F 85` | IDM「get WiFi Adapter MAC」（type2 身份） |
| `idmruntime.dll` | `83 7b 64 47 75` | IDM 另一条取 MAC 路径 |

只要三处一致，手机就把两套合并为单设备；旧补丁只改了第一处，导致三处身份分歧，进而出现重复设备。`0x06` 只是「以太网类型」并非「当前活跃网卡」，而 Windows 没有可直接读取的「活跃出口网卡」字段（需遍历比较 `Ipv4Metric`，无法等长替换实现）。

**方案**：把三处统一改为同一介质，由用户按接入方式二选一：

- `--mode wifi`：三处 = `0x47`（等同出厂，单设备、走 WiFi）
- `--mode lan`：三处 = `0x06`（统一以太网，单设备、走有线）

### 双网卡同网段的媒体会话路由

仅替换上述三处会让广播身份使用有线 MAC，但实际 WFD 音频会话由 `MiPlayCastService.exe` 子进程建立。抓取日志可见故障场景已完成发现、认证与 `SETUP`，PC 发出 `PLAY` 后手机立刻发送 `wfd_trigger_method: TEARDOWN`，且没有 RTP 音频包。根因是有线与 Wi-Fi 同时在线且同一 IPv4 子网时，Windows 因有线跃点更低，令该会话从有线接口出站，手机发现身份与媒体接口不一致而拒绝播放。

因此 `audio apply --mode lan` 默认会新增一条由工具记录和管理的 **持久 Wi-Fi 本地子网路由**（路由跃点为 1）。它只匹配 Wi-Fi 的本地 IPv4 前缀；互联网默认路由继续按用户原有的有线跃点选择。切回 `--mode wifi` 或执行 `audio revert` 会只删除本工具创建的该条路由。必要时可用 `--no-wifi-local-route` 关闭此行为。

定位采用指令块特征（含其后的 `jne`，保证唯一）而非硬编码偏移，已在版本升级（5.8.0.14 -> 5.8.0.74）后验证仍精确命中。等长字节替换、幂等、可还原；WiFi 态输出与出厂逐字节一致。

代码：[`src/mipcaudio_lan.rs`](src/mipcaudio_lan.rs)

## 补丁四：设备伪装（DeviceSpoof）

**目标**：在 `XiaomiPcManager.exe` 同目录释放代理 `msimg32.dll` + 写入注册表机型。

**原理**：利用 DLL 搜索顺序，同目录的 `msimg32.dll` 优先于系统目录被加载。该代理 DLL 读取 `HKCU\Software\SmartSharePatch\SpoofDevice` 的机型代号并据此伪装本机型号。

**实现**：`msimg32.dll` 已通过 `include_bytes!` 内嵌进编译产物，应用时直接释放到版本目录，无需附带文件；同时写入注册表机型代号。还原时删除该 DLL 并移除注册表项（若原目录本就存在同名文件，则在应用时备份、还原时恢复）。

预置机型（亦可 `--model` 自定义任意代号）：

| 代号 | 机型 |
|---|---|
| `TM2424`（默认） | Xiaomi Book Pro 14 (2026) |
| `TM2309` | Redmi Book 16 (2024) |

代码：[`src/device_spoof.rs`](src/device_spoof.rs)

> dll来源 @ChsBuffer

## 安装小米电脑管家

安装入口仅在未检测到 `PcContinuity` 时可用。工具优先扫描 Patcher 可执行文件同目录的 `*_XiaomiPCManager_*.exe`；也可接受本地 `.exe` 路径，或调用 Windows PowerShell `Invoke-WebRequest` 将 HTTP(S) URL 下载到该目录。URL 与目标路径通过子进程环境变量传入，不会拼接到 PowerShell 脚本中。下载先写入 `.download.tmp` 临时文件，成功后再重命名，避免保留不完整安装包。

启动安装包前，复用 DeviceSpoof 的内嵌代理释放逻辑，将 `msimg32.dll` 写入安装包同目录。若目标已存在且内容不同，会先创建 `.orig.bak` 备份。

## 构建

```text
cargo build --release
cargo test
```

release 产物路径为 `target/release/mipcm_patch.exe`。

release 产物会嵌入 `resources/mipcm_patch.exe.manifest`，其中声明 `requestedExecutionLevel=requireAdministrator`。因此从资源管理器双击 `mipcm_patch.exe` 时，Windows 会在程序启动前弹出 UAC。

> 注：`src/device_spoof.rs` 通过 `include_bytes!` 内嵌 `src/dlls/msimg32.dll`，构建时该文件需存在。
