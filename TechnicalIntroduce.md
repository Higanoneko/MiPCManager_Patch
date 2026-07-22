# Technical Introduce

本文档记录 MiPCManager Patcher 的补丁原理、定位方式、实现细节与构建说明。面向普通用户的功能说明见 [README.md](README.md)。

## 项目概览

工具提供两个前端入口，共享同一核心库：

| 产物 | 入口 | 说明 |
|---|---|---|
| `MiPCM_GUI_v*.*.*.exe` | `src/ui/gui/main.rs` | egui 图形界面，支持拖放安装 |
| `MiPCM_CLI_v*.*.*.exe` | `src/main.rs` | clap 命令行 + 交互菜单 |

核心库 (`src/lib.rs`) 将各模块聚合为 `ops` 层的高层操作，确保两个前端调用完全相同的逻辑。

## 总体设计

工具自动探测 `XiaomiPCManager` 与 `PcContinuity` 的最新安装版本：

- **XiaomiPCManager**（完整版）：位于 `C:\Program Files\MI\XiaomiPCManager`，支持所有补丁功能。工具启动时自动关闭其相关进程。
- **PcContinuity**（小米互联）：位于 `C:\Program Files\MI\PcContinuity`，目前**仅支持地区伪装**。不做启动时全量进程关闭。

各补丁动作执行前按功能关闭对应进程作为兜底：

| 补丁 | 关闭进程 |
|---|---|
| 地区伪装 | `micont_service.exe` |
| 摄像头弹窗 | `XiaomiPcManager.exe` |
| 音频流转 | `MiPCAudio.exe`、`MiPlayCastService.exe`、`MAFSvr.exe` |
| 设备伪装 | `XiaomiPcManager.exe` |

补丁前自动备份原文件（`.orig` 后缀），所有补丁幂等且可还原。若 Patch/还原时遇到 access denied 错误（`os error 5`），会自动关闭对应进程并重试一次。

修改 `Program Files` 下文件需管理员权限，release exe 内嵌 `requireAdministrator` manifest，双击启动即弹 UAC，运行时仍保留提权兜底。可通过环境变量 `MIPCM_NO_ELEVATE=1` 跳过运行时提权兜底（但 Release manifest 强制提权不会被跳过）。

## 补丁一：LocaleSpoof（地区伪装）

**目标**：`micont_rtm.dll`（原生 PE64）

**原理**：该 DLL 原本读取 `HKCU\Control Panel\International\Geo\Name`（系统真实地区）。对比作者提供的 `Original` 与 `Patched` 发现全文件仅 4 字节差异，集中在一处宽字符串：紧邻 `...International\Geo\0` 之后的值名 `Name` 被改为 `XCN`。

| 文件偏移 | Original (UTF-16LE) | Patched (UTF-16LE) |
|---|---|---|
| `Geo\0` 之后 | `N a m e \0`（`4E00 6100 6D00 6500 0000`） | `X C N \0 \0`（`5800 4300 4E00 0000 0000`） |

于是程序改读同一注册表键下的 `XCN` 值；工具再向该键写入 `XCN=CN`，程序便读到地区 `CN`，而系统真实 `Name` 保持不变。

**实现**：以宽字符串 `Geo\0` 作锚点，把其后 10 字节的 `Name\0\0` 等长替换为 `XCN\0\0`（不移位、不依赖偏移）。本工具输出与作者黄金参考 `micont_rtm.patched.dll` 逐字节一致（已验证）。

**自动探测**：优先使用 `XiaomiPCManager` 的最新版本；如未找到可用目标，则使用 `PcContinuity` 的最新版本。

**命令行选项**：
- `--region`：指定地区值，默认 `CN`
- `--no-registry`：不写入地区注册表值
- `--no-kill`：不自动关闭相关进程

代码：[`src/patches/locale_spoof.rs`](src/patches/locale_spoof.rs)

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

1. 纯 Rust 解析 ECMA-335 元数据，按「类型名 + 方法名后缀」定位 `MethodDef`，取得方法体 RVA 与 RVA 字段的文件偏移（[`src/infra/dotnet/metadata.rs`](src/infra/dotnet/metadata.rs)）。
2. 解析方法体（fat/tiny 头、EH 段），在 IL 前拼接 5 字节守卫 `ldarg.1; ldc.i4.3; bne.un.s +1; ret`，必要时整体修正 EH 偏移（[`src/infra/dotnet/method_body.rs`](src/infra/dotnet/method_body.rs)）。
3. 追加一个新节 `.mipatch` 写入新方法体，丢弃失效的 Authenticode 证书、维护 `SizeOfImage`、重算 PE 校验和，并把 `MethodDef.RVA` 改指到新节（[`src/infra/dotnet/pe.rs`](src/infra/dotnet/pe.rs)）。

**验证**：补丁后程序集可被 ILSpy 正常反编译，`ExceptionCallback` 反编译结果即为上述守卫；PE 结构、元数据、方法体头部（codesize/maxstack/局部签名）均合法；重复执行幂等。

代码：[`src/patches/camera_toast.rs`](src/patches/camera_toast.rs)、[`src/infra/dotnet/`](src/infra/dotnet/)

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

**命令行选项**：
- `--mode wifi|lan`：选择网络介质
- `--no-wifi-local-route`：有线广播时不添加 Wi-Fi 本地子网优先路由
- `--no-kill`：不自动关闭相关进程

代码：[`src/patches/mipcaudio_lan.rs`](src/patches/mipcaudio_lan.rs)

## 补丁四：设备伪装（DeviceSpoof）

**目标**：在 `XiaomiPcManager.exe` 同目录释放代理 `msimg32.dll` + 写入注册表机型。

**原理**：利用 DLL 搜索顺序，同目录的 `msimg32.dll` 优先于系统目录被加载。该代理 DLL 读取 `HKCU\Software\SmartSharePatch\SpoofDevice` 的机型代号并据此伪装本机型号。

**实现**：`msimg32.dll` 已通过 `include_bytes!` 内嵌进编译产物（`src/infra/dlls/msimg32.dll`），应用时直接释放到版本目录，无需附带文件；同时写入注册表机型代号。还原时删除该 DLL 并移除注册表项（若原目录本就存在同名文件，则在应用时备份、还原时恢复）。

**预置机型**（亦可 `--model` 自定义任意代号）：

| 代号 | 机型 |
|---|---|
| `TM2424`（默认） | Xiaomi Book Pro 14 (2026) |
| `TM2309` | Redmi Book 16 (2024) |

**命令行选项**：
- `--model`：指定伪装机型，默认 `TM2424`

代码：[`src/patches/device_spoof.rs`](src/patches/device_spoof.rs)

> DLL 来源：@ChsBuffer

## 安装小米电脑管家

安装入口仅在未检测到 `PcContinuity` 时可用（官方不允许两者同时安装）。工具优先扫描 Patcher 可执行文件同目录的 `*_XiaomiPCManager_*.exe`；找到一个时直接使用，找到多个时请用户选择。如未找到，则提示输入 HTTP(S) 网址或本地 `.exe` 路径。

启动安装包前，复用 DeviceSpoof 的内嵌代理释放逻辑，将 `msimg32.dll` 写入安装包同目录。若目标已存在且内容不同，会先创建 `.orig.bak` 备份。

**URL 下载**：调用 Windows PowerShell `Invoke-WebRequest`，URL 通过子进程环境变量传入（不拼接到 PowerShell 脚本中）。下载先写入 `.download.tmp` 临时文件，成功后再重命名，避免保留不完整安装包。

**命令行选项**：
- `--installer <exe>`：显式指定安装包路径
- `--url <url>`：通过 HTTP(S) 下载安装包

## GUI 实现

GUI 使用 egui + eframe 的 glow 后端构建，轻量且体积可控。主要布局：

- **安装状态区**：显示当前安装位置和各补丁状态
- **补丁操作区**：应用 / 还原按钮，含机型选择下拉框和自定义输入
- **安装拖放区**：支持拖入 `.exe` 安装包
- **日志区**：实时显示操作日志

窗口尺寸 760×1050，最小 640×820，居中显示。通过 `ico` crate 解析 `assets/MiPCManager.ico` 作为窗口图标。

## 代码结构

```
src/
├── lib.rs              # 核心库入口，聚合各模块
├── ops.rs              # 高层操作（apply/revert/status）
├── elevate.rs          # 管理员提权兜底
├── infra/
│   ├── dotnet/         # ECMA-335 元数据解析、方法体注入、PE 重写
│   └── dlls/           # 内嵌 DLL 资源
├── patches/
│   ├── locale_spoof.rs      # 地区伪装
│   ├── camera_toast.rs      # 摄像头弹窗抑制
│   ├── mipcaudio_lan.rs     # 音频流转
│   └── device_spoof.rs      # 设备伪装
├── install/
│   └── mod.rs          # 安装逻辑
├── experimental/
│   └── mod.rs          # 实验性功能（双网卡修复、SMBIOS 伪装）
├── ui/
│   ├── gui/main.rs     # egui 图形界面
│   ├── tui.rs          # ratatui 终端 UI
│   └── mod.rs
├── main.rs             # CLI 入口
└── bin/                # 命令行解析
```

## 构建

```text
cargo build --release
cargo test
```

release 产物路径：
- `target/release/MiPCM_CLI.exe`
- `target/release/MiPCM_GUI.exe`

release 产物会嵌入 `resources/mipcm_patch.exe.manifest` 与 `resources/mipcm_gui.exe.manifest`，其中声明 `requestedExecutionLevel=requireAdministrator`。因此从资源管理器双击 exe 时，Windows 会在程序启动前弹出 UAC。

构建时 `src/patches/device_spoof.rs` 通过 `include_bytes!` 内嵌 `src/infra/dlls/msimg32.dll`，该文件需存在。

可通过 `MIPCM_SKIP_GUI_MANIFEST=1` 跳过 GUI manifest 嵌入（使用 `mipcm_gui_test.rc`，不强制管理员，便于本机无 UAC 冒烟测试）。

**构建配置**（`Cargo.toml` release profile）：

| 选项 | 值 | 说明 |
|---|---|---|
| `opt-level` | `z` | 优化体积 |
| `lto` | `true` | 链接时优化 |
| `strip` | `true` | 剥离调试符号 |
| `panic` | `abort` | 减少 unwind 代码 |
