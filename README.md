![alt text](MiPCMPatcher.png)
# MiPCManager Patcher

为「小米电脑管家 (XiaomiPCManager)」提供功能增强与还原能力的补丁工具。

本项目适合希望在非官方支持场景下继续使用部分小米电脑管家能力的用户。工具会自动查找小米电脑管家的安装版本，在修改前备份原文件，并支持重复执行与还原。

同时支持自动探测 `C:\Program Files\MI\PcContinuity`；该安装类型目前**仅支持地区伪装**，摄像头弹窗、音频流转和设备伪装不会对它开放。

## 能做什么

- 地区伪装：让小米电脑管家读取指定地区值，默认伪装为 `CN`。
- 抑制摄像头误报弹窗：屏蔽「请确认摄像头状态」这类本机摄像头被误判禁用时的弹窗。
- 音频流转增强：可在无线 WiFi 与有线 LAN 两种模式之间切换。
- 设备伪装：写入指定机型代号，并释放代理 DLL，让小米电脑管家识别为指定设备型号。
- 安装小米电脑管家：自动查找或下载安装包，释放内嵌 `msimg32.dll` 后启动安装。
- 状态查看：检查当前安装位置、目标文件和各补丁状态。
- 一键还原：每个功能都提供对应的 `revert` 操作。

## 使用方式

修改 `Program Files` 下的文件需要管理员权限。Release 版 exe 已内嵌管理员权限清单，双击运行时会弹出 UAC。

无参数运行会进入交互菜单：

```text
=== 小米电脑管家增强补丁 ===
  1) 查看状态     2) 地区伪装       3) 抑制摄像头弹窗
  4) 音频流转增强 5) 设备伪装       6) 安装小米电脑管家
  0) 退出
```

也可以通过命令行使用：

```text
mipcm_patch status
mipcm_patch locale apply | revert
mipcm_patch camera apply | revert
mipcm_patch audio  apply --mode wifi|lan [--no-wifi-local-route] | revert
mipcm_patch device apply --model TM2424 | revert
mipcm_patch install [--installer <exe> | --url <url>]
```

常用选项：

- `--dll <路径>`：为 `locale` / `camera` 指定目标 DLL。
- `--dir <版本目录>`：为 `audio` / `device` 指定小米电脑管家版本目录。
- `--no-kill`：不自动关闭相关进程。
- `audio --no-wifi-local-route`：有线广播时不添加 Wi-Fi 本地子网优先路由；仅在明确不需要双网卡同网段修复时使用。
- `locale --region <地区>`：指定地区值，默认 `CN`。
- `locale --no-registry`：不写入地区注册表值。
- `device --model <机型代号>`：指定伪装机型，默认 `TM2424`。
- `install --installer <exe>`：显式指定小米电脑管家安装包。
- `install --url <url>`：使用 Windows PowerShell `Invoke-WebRequest` 下载 HTTP(S) 安装包到 Patcher 同目录。

## 功能说明

### 地区伪装

让小米电脑管家读取工具写入的地区值，而不是直接读取系统真实地区。默认写入 `CN`，不会修改系统原本的地区设置。

自动探测时优先使用 `XiaomiPCManager` 的最新版本；如未找到可用目标，则使用 `PcContinuity` 的最新版本。

### 抑制摄像头弹窗

只针对「本机摄像头被误判禁用」这一类提示进行抑制。权限提示、摄像头占用、连接断开等其它提示会保留。

### 音频流转增强

用于切换音频流转使用的网络介质：

- `--mode wifi`：恢复/使用无线模式。
- `--mode lan`：使用有线模式。

使用 `--mode lan` 且有线、Wi-Fi 同时在线时，工具会额外创建一条由工具管理的 Wi-Fi 本地子网路由。它只让到手机所在局域网的音频会话从 Wi-Fi 网卡返回，默认路由仍保持有线优先；`audio revert` 会移除该路由。

如果手机端反复切换后仍显示重复设备或连接状态异常，可以在手机端将该电脑移除/忘记后重新配对。

### 设备伪装

默认伪装为 `TM2424`，也可以通过 `--model` 指定其它机型代号。

预置机型：

| 代号 | 机型 |
|---|---|
| `TM2424` | Xiaomi Book Pro 14 (2026) |
| `TM2309` | Redmi Book 16 (2024) |

### 安装小米电脑管家

该功能在未安装任何产品，或已安装 `XiaomiPCManager` 时可用；检测到 `PcContinuity` 时不可用，因为官方不允许两者同时安装。

未显式指定安装包时，工具会先在 Patcher 可执行文件同目录查找 `*_XiaomiPCManager_*.exe`。找到一个时直接使用，找到多个时请用户选择；如未找到，则提示输入 HTTP(S) 网址或本地 `.exe` 路径。启动安装包前，会将内嵌 `msimg32.dll` 释放到安装包所在目录。

## 注意事项

- 完整版 `XiaomiPCManager` 被探测到时，工具启动会先关闭其相关进程。`PcContinuity` 不做启动时全量关闭，只在地区伪装前处理 `micont_service.exe`。补丁后请手动重新打开相关程序。
- 各补丁动作执行前仍会按功能关闭对应进程作为兜底：地区伪装关闭 `micont_service.exe`，摄像头弹窗和设备伪装关闭 `XiaomiPcManager.exe`，音频流转关闭 `MiPCAudio.exe`、`MiPlayCastService.exe` 与 `MAFSvr.exe`。
- 地区伪装 Patch 前必须确保 `micont_service.exe` 已退出；若使用 `--no-kill` 且该进程仍在运行，工具会拒绝继续 Patch。
- `PcContinuity` 安装目录下只允许地区伪装；即使通过 `--dll` 或 `--dir` 显式指定其中的路径，其他补丁也会被拒绝。
- 所有 Patch/还原操作若遇到 `拒绝访问。 (os error 5)`，工具会自动关闭对应进程并重试一次。
- 如需跳过运行时提权兜底，可设置环境变量 `MIPCM_NO_ELEVATE=1`；Release 版本的 exe 的 manifest 强制提权不会被该环境变量跳过。

## 技术说明

补丁定位、PE/.NET 修改方式、构建说明等技术细节见 [TechnicalIntroduce.md](TechnicalIntroduce.md)。

## 感谢列表

- @ChsBuffer 的 `msimg32.dll` 为本项目提供了 DLL 代理支持。
- Coolapk@Na1veMagic 的技术思路为本项目的 `LocaleSpoof` 提供了重要参考。

## 免责声明

本工具所用所有图标（特指 assets/MiPCManager.ico）均归北京小米移动软件有限公司所有，受相关版权法律保护。未经授权，禁止任何形式的复制、分发、展示或使用这些图标。

本工具仅供学习和研究使用，作者不对因使用本工具而导致的任何直接或间接损失承担责任。使用者应自行承担使用本工具的风险，并确保其行为符合当地法律法规。
