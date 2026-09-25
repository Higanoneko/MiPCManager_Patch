# Technical Introduce

本文档记录 MiPCManager Patcher 对小米电脑管家、小米互联 / 互联互通和超级小爱执行的操作，以及补丁原理、定位方式、实现细节与构建说明。面向普通用户的功能说明见 [README.md](README.md)。

## 项目概览

工具提供两个前端入口，共享同一核心库：

| 产物 | 入口 | 说明 |
|---|---|---|
| `MiPCM_GUI_v*.*.*.exe` | `src/ui/gui/app.rs` | Slint 图形界面，支持选择本地安装包或输入下载地址 |
| `MiPCM_CLI_v*.*.*.exe` | `src/main.rs` | clap 命令行；无参数启动 ratatui 交互界面 |

核心库 (`src/lib.rs`) 将各模块聚合为 `ops` 层的高层操作，确保两个前端调用完全相同的逻辑。

## 总体设计

工具自动探测 `XiaomiPCManager`、小米互联 / 互联互通（`PcContinuity` / `HyperConnect`）与超级小爱（`XiaoaiAgent`）的最新安装版本：

- **XiaomiPCManager**（完整版）：位于 `C:\Program Files\MI\XiaomiPCManager`，支持所有补丁功能。工具启动时自动关闭其相关进程。
- **PcContinuity**（小米互联）：位于 `C:\Program Files\MI\PcContinuity`，目前**仅支持地区伪装**。
- **HyperConnect**（小米互联互通 2.0）：位于 `C:\Program Files\MI\HyperConnect`，原生互联 DLL 在版本目录下的 `resources\native-interconnect\win32` 子目录（`micont_rtm.dll`、`micont_service.exe` 等），同样**仅支持地区伪装**。
- **XiaoaiAgent**（超级小爱）：位于 `C:\Program Files\MI\XiaoaiAgent`，工具从其子目录中选择版本号最高的目录，不对具体版本设置适配限制。

小米互联 / 互联互通两个产品均不做启动时全量进程关闭（仅按功能关闭对应进程）。

各补丁动作执行前按功能关闭对应进程作为兜底：

| 补丁 | 关闭进程 |
|---|---|
| 地区伪装 | `micont_service.exe` |
| 摄像头弹窗 | `XiaomiPcManager.exe` |
| 鼠标快捷键 | `XiaomiPcManager.exe`、`XiaomiPcHost.exe` |
| 音频流转 | `MiPCAudio.exe`、`MiPlayCastService.exe`、`MAFSvr.exe` |
| 设备伪装 | `XiaomiPcManager.exe` |
| 超级小爱 | `XiaoaiAgent.exe` |

补丁前自动备份原文件（文件名后追加 `.orig.bak`），所有补丁幂等且可还原。若 Patch/还原时遇到 access denied 错误（`os error 5`），会自动关闭对应进程并重试一次。

修改 `Program Files` 下文件需管理员权限，release exe 内嵌 `requireAdministrator` manifest，双击启动即弹 UAC，运行时仍保留提权兜底。可通过环境变量 `MIPCM_NO_ELEVATE=1` 跳过运行时提权兜底（但 Release manifest 强制提权不会被跳过）。

## 补丁一：LocaleSpoof（地区伪装）

**目标**：`micont_rtm.dll`（原生 PE64）

**原理**：该 DLL 原本读取 `HKCU\Control Panel\International\Geo\Name`（系统真实地区）。对比作者提供的 `Original` 与 `Patched` 发现全文件仅 4 字节差异，集中在一处宽字符串：紧邻 `...International\Geo\0` 之后的值名 `Name` 被改为 `XCN`。

| 文件偏移 | Original (UTF-16LE) | Patched (UTF-16LE) |
|---|---|---|
| `Geo\0` 之后 | `N a m e \0`（`4E00 6100 6D00 6500 0000`） | `X C N \0 \0`（`5800 4300 4E00 0000 0000`） |

于是程序改读同一注册表键下的 `XCN` 值；工具再向该键写入 `XCN=CN`，程序便读到地区 `CN`，而系统真实 `Name` 保持不变。

**实现**：以宽字符串 `Geo\0` 作锚点，把其后 10 字节的 `Name\0\0` 等长替换为 `XCN\0\0`（不移位、不依赖偏移）。本工具输出与作者黄金参考 `micont_rtm.patched.dll` 逐字节一致（已验证）。

**自动探测**：优先使用 `XiaomiPCManager` 的最新版本；如未找到可用目标，则使用小米互联 / 互联互通（`HyperConnect` 优先，`PcContinuity` 兜底）的最新版本。`HyperConnect` 2.0 的 DLL 位于版本目录的 `resources\native-interconnect\win32` 子目录，工具会自动定位（已针对 2.0.0.429 验证特征仍命中）。

**命令行选项**：
- `--region`：指定地区值，默认 `CN`
- `--no-registry`：不写入地区注册表值
- `--no-kill`：不自动关闭相关进程

代码：[`src/patches/locale/mod.rs`](src/patches/locale/mod.rs)

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

1. 纯 Rust 解析 ECMA-335 元数据，按「类型名 + 方法名后缀」定位 `MethodDef`，取得方法体 RVA 与 RVA 字段的文件偏移（[`src/patches/camera/dotnet/metadata.rs`](src/patches/camera/dotnet/metadata.rs)）。
2. 解析方法体（fat/tiny 头、EH 段），在 IL 前拼接 5 字节守卫 `ldarg.1; ldc.i4.3; bne.un.s +1; ret`，必要时整体修正 EH 偏移（[`src/patches/camera/dotnet/method_body.rs`](src/patches/camera/dotnet/method_body.rs)）。
3. 追加一个新节 `.mipatch` 写入新方法体，丢弃失效的 Authenticode 证书、维护 `SizeOfImage`、重算 PE 校验和，并把 `MethodDef.RVA` 改指到新节（[`src/infra/pe.rs`](src/infra/pe.rs)）。

**验证**：补丁后程序集可被 ILSpy 正常反编译，`ExceptionCallback` 反编译结果即为上述守卫；PE 结构、元数据、方法体头部（codesize/maxstack/局部签名）均合法；重复执行幂等。

代码：[`src/patches/camera/mod.rs`](src/patches/camera/mod.rs)、[`src/patches/camera/dotnet/`](src/patches/camera/dotnet/)、[`src/infra/pe.rs`](src/infra/pe.rs)

## 补丁三：鼠标快捷键（含两个侧键）

**目标**：`dist/static/js/main.js`（小米电脑管家设置页的压缩 Web 资源）

快捷键配置链路由三层组成：设置页通过 `hot_key` 字段提交由 `+` 分隔的按键名；`XiaomiPcManager.dll` 中的 `JsSetShareDesktopHandler` / `JsSetSearchFileHandler` 将其拆成最多三个字符串并交给 `MiSmartShareClrWrapper`；`MiScreenShare.exe` 中可见配置路径 `Software\Timi Personal Computing\MiScreenShare\ShortCutKey` 以及 `go_back`、`go_home`、`switch_task`、`open_clipboard`、`open_ai_search` 等动作名。实际快捷键值并非安装目录中的 INI 配置。

快捷键设置组件原本只在 `onMouseDown` 中处理中键和右键，分别向后端写入 `MMouseButtonDown` 与 `RMouseButtonDown`；“打开搜索”一项还通过 `disableMiddleKey` / `disableRightKey` 禁用了这两种输入。鼠标前进、后退侧键对应的浏览器 `MouseEvent.button` 值为 3、4，但原设置页未处理。

对 5.8.0.95 的原生程序分析确认，`MiScreenShare.exe` 已包含并处理 `XMouseButtonDown` 与 `XMouseButtonDown2`。因此补丁只需补齐设置页映射：

| 鼠标输入 | 配置值 |
|---|---|
| 中键 | `MMouseButtonDown` |
| 右键 | `RMouseButtonDown` |
| 侧键 1（后退） | `XMouseButtonDown` |
| 侧键 2（前进） | `XMouseButtonDown2` |

补丁还解除“打开搜索”对中键和右键的界面禁用。左键用于聚焦输入框且底层没有对应快捷键配置值，因此不纳入补丁。实现使用两组唯一文本特征，不依赖固定文件偏移；版本结构不匹配时会拒绝写入。原始 `main.js` 自动保存为 `.orig.bak`，重复应用幂等，还原时直接恢复备份。

代码：[`src/patches/hotkey/mod.rs`](src/patches/hotkey/mod.rs)

## 补丁四：MiPCAudio 音频流转「无线 / 有线」模式

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

代码：[`src/patches/audio/mod.rs`](src/patches/audio/mod.rs)

## 补丁五：设备伪装（DeviceSpoof）

**目标**：在 `XiaomiPcManager.exe` 同目录释放代理 `msimg32.dll` + 写入注册表机型。

**原理**：利用 DLL 搜索顺序，同目录的 `msimg32.dll` 优先于系统目录被加载。该代理 DLL 读取 `HKCU\Software\SmartSharePatch\SpoofDevice` 的机型代号并据此伪装本机型号。

**实现**：`msimg32.dll` 已通过 `include_bytes!` 内嵌进编译产物（`src/patches/device/dlls/msimg32.dll`），应用时直接释放到版本目录，无需附带文件；同时写入注册表机型代号。还原时删除该 DLL 并移除注册表项（若原目录本就存在同名文件，则在应用时备份、还原时恢复）。

**预置机型**（亦可 `--model` 自定义任意代号）：

| 代号 | 机型 |
|---|---|
| `TM2425`（默认） | Redmi Book Pro 14 (2026) |
| `TM2424` | Xiaomi Book Pro 14 (2026) |
| `TM2309` | Redmi Book 16 (2024) |

**命令行选项**：
- `--model`：指定伪装机型，默认 `TM2425`

代码：[`src/patches/device/mod.rs`](src/patches/device/mod.rs)

> DLL 来源：@ChsBuffer

## 安装小米电脑管家

安装入口仅在未检测到 `PcContinuity` 时可用（官方不允许两者同时安装）。工具优先扫描 Patcher 可执行文件同目录的 `*_XiaomiPCManager_*.exe`；找到一个时直接使用，找到多个时请用户选择。如未找到，则提示输入 HTTP(S) 网址或本地 `.exe` 路径。

启动安装包前，工具会在安装包同目录临时准备 `msimg32.dll`，写入默认伪装机型，然后挂起启动安装器、注入代理并旁路系统版本与机型检查。安装器启动成功后，安装包目录中的临时文件会恢复为操作前的状态。

**URL 下载**：调用 Windows PowerShell `Invoke-WebRequest`，URL 通过子进程环境变量传入（不拼接到 PowerShell 脚本中）。下载先写入 `.download.tmp` 临时文件，成功后再重命名，避免保留不完整安装包。

**命令行选项**：
- `--installer <exe>`：显式指定安装包路径
- `--url <url>`：通过 HTTP(S) 下载安装包

## 安装超级小爱并注入补丁

超级小爱安装流程复用通用的安装包查找、下载、目录探测、原子写入和备份能力，但使用独立的 `userenv.dll`，不会复用小米电脑管家的 `msimg32.dll`。

超级小爱安装器仅接受以下两个文件名（不区分大小写）；本地自动查找也只匹配这两个名称：

- `XiaoaiAgent_Setup.exe`
- `s6bK_XiaoaiAgent_3.5.0.220_31444585.exe`

文件名不用于推断最终安装版本。工具对超级小爱不设置具体版本限制；安装完成后才扫描 `C:\Program Files\MI\XiaoaiAgent\<版本目录>`，选择版本号最高的目录作为部署目标。

安装流程如下：

1. 保存安装包目录中原有 `userenv.dll` 的状态，并临时释放本工具内嵌的超级小爱专用 DLL。
2. 启动安装器，跟踪安装器进程及其派生的安装进程，等待用户完成安装并关闭安装窗口；安装后自动启动、且位于 XiaoaiAgent 安装目录内的程序不计入安装器进程树等待。
3. 无论安装成功或失败，都尝试将安装包目录恢复为操作前状态。
4. 安装成功后探测实际版本目录，关闭正在运行的 `XiaoaiAgent`，再把专用 `userenv.dll` 部署到该目录。
5. 输出重启提示；工具不会自行重启电脑。

应用补丁时，若目标目录原本存在不同的 `userenv.dll`，会先保存为 `userenv.dll.orig.bak`；若原本不存在，则创建空的同名备份标记。还原时据此恢复原文件或删除本工具部署的 DLL，并且在文件内容不属于本工具时拒绝删除。重复应用会识别已部署状态并跳过写入。

**命令行入口**：

- `xiaoai install`：安装 Patcher 同目录中唯一一个已识别的超级小爱安装包
- `xiaoai install --installer <exe>`：显式指定安装包
- `xiaoai install --url <url>`：通过用户提供的 HTTP(S) 地址下载安装包
- `xiaoai apply [--dir <版本目录>]`：向已安装目录部署补丁
- `xiaoai revert [--dir <版本目录>]`：还原补丁

代码：[`src/install/xiaoai_installer.rs`](src/install/xiaoai_installer.rs)、[`src/patches/ai/mod.rs`](src/patches/ai/mod.rs)、[`src/ops.rs`](src/ops.rs)

## 本程序所做的操作

所有探测、补丁、安装和还原操作均在本机执行。程序不会上传设备信息、补丁状态或文件内容；只有用户明确提供 HTTP(S) 安装包地址时，才会调用系统 Windows PowerShell 下载该文件。

| 类型 | 程序行为 | 还原方式 |
|---|---|---|
| 安装目录与状态探测 | 枚举 `C:\Program Files\MI` 下受支持产品的版本目录，读取目标文件以判断补丁状态 | 只读，无需还原 |
| 进程管理 | 应用或还原补丁前，按功能结束可能占用目标文件的相关进程 | 用户可重新启动对应程序；超级小爱按提示重启电脑 |
| 地区伪装 | 修改 `micont_rtm.dll` 中读取的值名，并写入 `HKCU\Control Panel\International\Geo\XCN` | 从 `.orig.bak` 恢复 DLL，并删除 `XCN` 值 |
| 摄像头弹窗 | 为 `PcControlCenter.dll` 追加 `.mipatch` 节并改写目标方法 RVA | 从 `.orig.bak` 恢复原 DLL |
| 鼠标快捷键 | 修改 `dist/static/js/main.js`，捕获两个侧键并解除搜索项的中键/右键禁用 | 从 `main.js.orig.bak` 恢复原文件 |
| 音频流转 | 等长修改 `MiPCAudio.exe` 与 `idmruntime.dll` 的三处网卡类型判断 | 从 `.orig.bak` 恢复原文件 |
| 有线音频路由 | 在有线模式下按需创建 metric=1 的持久 Wi-Fi 本地子网路由，并在版本目录记录 `.mipcm_audio_wifi_route` | 只删除本工具有状态记录的路由和状态文件 |
| 设备伪装 | 向小米电脑管家版本目录部署 `msimg32.dll`，并写入 `HKCU\Software\SmartSharePatch\SpoofDevice` | 恢复或删除代理 DLL，并删除注册表值 |
| 超级小爱 | 安装时临时部署、随后恢复安装包目录中的 `userenv.dll`；安装后向实际版本目录部署该 DLL | 根据 `.orig.bak` 恢复原文件，或删除本工具部署的 DLL |
| 安装包下载 | 把用户指定 URL 下载到 Patcher 目录的 `.download.tmp`，成功后再重命名为 `.exe`；不会覆盖已有目标 | 用户可自行删除已下载安装包 |
| 产品卸载 | 经用户确认后运行产品自带卸载程序；相关入口还可删除已知服务、残留目录或 MiDrop Ext MSIX，并在需要时重启资源管理器 | 属于不可逆操作，执行前由界面要求确认 |

除产品卸载外，补丁操作均以幂等和可还原为目标。对目标文件的持久写入使用同目录临时文件替换；若已有备份，程序保留首次备份，不覆盖原始副本。

## GUI 实现

GUI 使用 Slint 声明式界面构建。主要布局：

- **安装状态区**：显示当前安装位置和各补丁状态
- **补丁操作区**：应用 / 还原按钮，含机型选择下拉框和自定义输入
- **安装区**：支持选择本地 `.exe` 安装包或输入下载地址，超级小爱安装任务在后台等待安装器完成
- **日志区**：实时显示操作日志

界面声明位于 `src/ui/app.slint`，Rust 侧事件与异步任务编排位于 `src/ui/gui/app.rs`。

## 代码结构

```
src/
├── lib.rs                       # 核心库入口，聚合各模块
├── ops.rs                       # 高层操作（apply/revert/status/install）
├── elevate.rs                   # 管理员提权兜底
├── infra/                       # PE、字节、注册表、PowerShell 基础设施
├── patches/
│   ├── locale/mod.rs            # 地区伪装
│   ├── camera/                  # 摄像头弹窗抑制与 .NET 方法体处理
│   ├── hotkey/mod.rs            # 鼠标中键、右键与侧键快捷键
│   ├── audio/mod.rs             # 音频流转与 Wi-Fi 本地路由
│   ├── device/                  # 设备伪装及内嵌 msimg32.dll
│   └── ai/                      # 超级小爱及内嵌 userenv.dll
├── install/
│   ├── mod.rs                   # 通用安装目录、进程和文件操作
│   ├── pc_manager_installer.rs  # 小米电脑管家安装
│   └── xiaoai_installer.rs      # 超级小爱安装
├── uninstall/mod.rs             # MSIX 与产品卸载
├── experimental/               # 实验性 SMBIOS 等功能
├── ui/
│   ├── app.slint                # GUI 界面声明
│   ├── gui/app.rs               # GUI 二进制入口与事件编排
│   └── tui/                     # ratatui 终端 UI
└── main.rs                      # CLI / TUI 统一入口
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

构建时 `src/patches/device/mod.rs` 与 `src/patches/ai/mod.rs` 分别通过 `include_bytes!` 内嵌 `src/patches/device/dlls/msimg32.dll` 和 `src/patches/ai/dlls/userenv.dll`，两个文件均需存在。

可通过 `MIPCM_SKIP_GUI_MANIFEST=1` 跳过 GUI manifest 嵌入（使用 `mipcm_gui_test.rc`，不强制管理员，便于本机无 UAC 冒烟测试）。

**构建配置**（`Cargo.toml` release profile）：

| 选项 | 值 | 说明 |
|---|---|---|
| `opt-level` | `z` | 优化体积 |
| `lto` | `true` | 链接时优化 |
| `strip` | `true` | 剥离调试符号 |
| `panic` | `abort` | 减少 unwind 代码 |

## 致谢

- Coolapk @Na1veMagic：地区伪装实现思路
- @ChsBuffer：设备伪装所用 `msimg32.dll`
- 感谢提供超级小爱专用 `userenv.dll`、整理安装补丁教程并完成实机验证的社区用户
