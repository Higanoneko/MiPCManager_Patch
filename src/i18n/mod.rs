//! 系统语言检测与界面文本国际化。
//!
//! 所有面向用户的界面文本（CLI、TUI、GUI）通过 [`detect_lang`] 检测系统语言后，
//! 由 [`tr`] 查找对应翻译。中文（含 zh-CN/zh-TW/zh-HK/zh-SG 等变体）返回中文，
//! 其他语言返回英文。
//!
//! 设计原则：
//! - 零分配：所有翻译均为 `&'static str`，无运行时内存分配。
//! - [`ops`] 返回的日志行保持中文不变（日志为技术信息，面向调试而非普通用户）。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

/// 检测系统语言。
///
/// Windows：调用 `GetUserDefaultUILanguage`，中文 LCID 包括
/// 0x0804(zh-CN)、0x0404(zh-TW)、0x0c04(zh-HK)、0x1004(zh-SG) 等。
/// 非 Windows：读取 `LANG` 环境变量，`zh*` 视为中文。
pub fn detect_lang() -> Lang {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        // 简体/繁体中文及常见 zh 变体 LCID
        if lang_id == 0x0804
            || lang_id == 0x0404
            || lang_id == 0x0c04
            || lang_id == 0x1004
        {
            return Lang::Zh;
        }
        // 兜底：检查系统 UI 语言是否为中文。
        // 某些 OEM/Region 变体可能上报其他 LCID 但仍是中文系统。
        // 通过 GEO 或 LANGID 主语言来判断。
        let primary = lang_id & 0x03FF;
        if primary == 0x0004 {
            return Lang::Zh;
        }
        Lang::En
    }
    #[cfg(not(windows))]
    {
        let lang = std::env::var("LANG").unwrap_or_default();
        if lang.to_lowercase().starts_with("zh") {
            Lang::Zh
        } else {
            Lang::En
        }
    }
}

/// 根据 key 和语言返回翻译字符串。未匹配的 key 原样返回。
pub fn tr(key: &str, lang: Lang) -> &str {
    match lang {
        Lang::Zh => tr_zh(key),
        Lang::En => tr_en(key),
    }
}

macro_rules! tr_table {
    ($lang:ident, { $($key:expr => $val:expr),* $(,)? }) => {
        fn $lang(key: &str) -> &str {
            match key {
                $($key => $val,)*
                _ => key,
            }
        }
    };
}

tr_table!(tr_zh, {
    // ── 通用 ──
    "app.title" => "MiPCM Patch",
    "btn.apply" => "应用",
    "btn.revert" => "还原",
    "btn.refresh" => "刷新",
    "btn.cancel" => "取消",
    "btn.wifi" => "WiFi",
    "btn.lan" => "LAN",
    "btn.diagnose" => "诊断",
    "btn.fix" => "修复",
    "btn.select" => "选择",
    "btn.start-install" => "开始安装",
    "btn.confirm" => "确认卸载",
    "btn.clear-log" => "清空日志",
    "btn.browse" => "选择…",
    "hint.custom" => "自定义",
    "hint.model.code" => "机型代号",
    "hint.custom.confirmed" => "自定义机型已设置",
    "hint.custom.tui" => "c—自定义",
    "hint.preset.tui" => "m—切换预设",
    "hint.uninstall.msix" => "检测并移除 MiDrop Ext MSIX 包，完成后重启资源管理器",
    "hint.uninstall.product" => "完整卸载主程序、子产品、移除服务、清理临时文件（不可逆！）",
    "hint.uninstall.warn" => "注意：卸载操作不可逆！",
    "hint.log.empty" => "操作日志将显示在这里…",
    "hint.install.tui" => "💡 安装请使用命令行：MiPCM_CLI install",

    // ── 面板/章节标题 ──
    "section.status" => "安装状态",
    "section.patches" => "补丁操作",
    "section.uninstall" => "卸载",
    "section.install" => "安装",
    "section.log" => "日志",
    "section.confirm.uninstall" => "确认卸载",
    "section.input.model" => "自定义机型 — {target_label}",
    "section.status.full" => "安装状态",

    // ── 补丁标签 ──
    "patch.locale" => "地区伪装",
    "patch.camera" => "摄像头弹窗",
    "patch.audio" => "音频流转",
    "patch.device" => "设备伪装",
    "patch.smbios" => "Lyra SMBIOS (Experimental)",
    "patch.dual-nic" => "Lyra 双网卡 (Experimental)",
    "patch.locale.detail" => "地区伪装 (micont_rtm.dll)",
    "patch.camera.detail" => "摄像头弹窗抑制 (PcControlCenter.dll)",
    "patch.audio.detail" => "音频流转广播模式",
    "patch.device.detail" => "设备伪装 (msimg32.dll)",
    "patch.smbios.detail" => "[实验性] SMBIOS 设备身份",

    // ── CLI 命令 ──
    "cli.about" => "小米电脑管家功能增强补丁工具",
    "cli.cmd.status" => "查看安装信息与补丁状态",
    "cli.cmd.locale" => "地区伪装（micont_rtm.dll）",
    "cli.cmd.camera" => "抑制「请确认摄像头状态」弹窗（PcControlCenter.dll）",
    "cli.cmd.audio" => "MiPCAudio 音频流转广播模式（无线/有线，统一身份并修复双网卡媒体路由）",
    "cli.cmd.device" => "设备伪装（释放 msimg32.dll + 写入机型注册表）",
    "cli.cmd.smbios" => "[实验性] SMBIOS 设备身份伪装 — 妙播 / Lyra 特殊适配（micont_rtm.dll IAT Hook）",
    "cli.cmd.install" => "安装小米电脑管家 / 小米互联（自动识别安装包所属产品）",
    "cli.cmd.uninstall" => "卸载 MiDrop Ext MSIX 包 或 小米电脑管家 / 小米互联",
    "cli.cmd.apply" => "应用补丁",
    "cli.cmd.revert" => "还原补丁",
    "cli.cmd.audio.apply" => "切换广播模式",
    "cli.cmd.audio.revert" => "还原音频流转补丁",
    "cli.cmd.audio.experimental" => "[实验性] 双网卡同网段音频修复：诊断并自动对齐 Wi-Fi 路由与广播模式",
    "cli.cmd.device.apply" => "应用设备伪装",
    "cli.cmd.device.revert" => "还原设备伪装",
    "cli.cmd.smbios.apply" => "应用 SMBIOS 设备身份伪装",
    "cli.cmd.smbios.revert" => "还原 SMBIOS 设备身份伪装",
    "cli.cmd.msix" => "卸载 MiDrop Ext MSIX 包（完成后重启资源管理器）",
    "cli.cmd.product" => "完整卸载小米电脑管家 / 小米互联（主程序 + 子产品 + 服务 + 文件清理，不可逆）",

    // ── CLI 参数帮助 ──
    "cli.arg.dll" => "指定目标 DLL 路径（默认自动探测安装目录）",
    "cli.arg.no_kill" => "不自动关闭相关进程",
    "cli.arg.region" => "伪装地区代码（默认 CN）",
    "cli.arg.no_registry" => "不修改注册表",
    "cli.arg.mode" => "广播介质：wifi（无线，默认）或 lan（有线）",
    "cli.arg.dir" => "指定版本目录（默认自动探测）",
    "cli.arg.no_wifi_route" => "不自动管理 Wi-Fi 本地子网优先路由（无线模式下用于修复双网卡同网段断流）",
    "cli.arg.dry_run" => "仅诊断，不执行修复",
    "cli.arg.model" => "机型代号（默认 TM2424）",
    "cli.arg.installer" => "显式指定 .exe 安装包",
    "cli.arg.url" => "从 HTTP(S) 地址下载安装包",

    // ── CLI 运行时消息 ──
    "cli.error" => "错误：{error}",
    "cli.confirm.uninstall" => "确认卸载？输入 y 继续：",
    "cli.cancelled.uninstall" => "已取消卸载。",
    "cli.cancelled.install" => "已取消安装。",
    "cli.downloading" => "正在下载安装包到 {path}",
    "cli.found.installer" => "已找到{kind}安装包：{path}",
    "cli.multiple.installers" => "找到多个安装包：",
    "cli.choose.installer" => "请选择安装包：",
    "cli.invalid.choice" => "无效的安装包选择",
    "cli.no.installer.found" => "未在 Patcher 同目录找到安装包（小米电脑管家 `*_XiaomiPCManager_*.exe` 或小米互联 `小米互联*.exe`）。",
    "cli.cancel.option" => "取消",
    "cli.choose.option" => "请选择：",
    "cli.menu.download_url" => "输入下载网址",
    "cli.menu.local_file" => "指定本地 .exe 安装包",
    "cli.prompt.url" => "请输入 HTTP(S) 下载地址：",
    "cli.prompt.path" => "请输入 .exe 安装包路径：",
    "cli.invalid.selection" => "无效选择",

    // ── 安装包分类 ──
    "install.kind.manager" => "小米电脑管家 (XiaomiPCManager)",
    "install.kind.continuity" => "小米互联 / 互联互通 (PcContinuity)",

    // ── 安装 GUI ──
    "install.title" => "安装小米电脑管家 / 小米互联",
    "install.download.placeholder" => "HTTP(S) 下载地址…",
    "install.file.placeholder" => "选择本地 .exe 安装包…",
    "install.download.and.install" => "下载并安装",

    // ── GUI 下载 ──
    "gui.downloading.start" => "开始下载：{url}",

    // ── GUI 安装操作标签 ──
    "gui.op.locale.apply" => "地区伪装",
    "gui.op.locale.revert" => "地区伪装·还原",
    "gui.op.device.apply" => "设备伪装({model})",
    "gui.op.device.revert" => "设备伪装·还原",
    "gui.op.camera.apply" => "摄像头",
    "gui.op.camera.revert" => "摄像头·还原",
    "gui.op.audio.wifi" => "音频·WiFi",
    "gui.op.audio.lan" => "音频·LAN",
    "gui.op.audio.revert" => "音频·还原",
    "gui.op.dualnic.diagnose" => "双网卡·诊断",
    "gui.op.dualnic.fix" => "双网卡·修复",
    "gui.op.smbios.apply" => "SMBIOS({model})",
    "gui.op.smbios.revert" => "SMBIOS·还原",
    "gui.op.uninstall.msix" => "卸载MSIX",
    "gui.op.uninstall.product" => "卸载产品",
    "gui.op.install" => "安装({path})",

    // ── GUI 警告 ──
    "gui.warn.no_full" => "未检测到完整版 XiaomiPCManager\n摄像头/音频/设备伪装不可用。",
    "gui.warn.recent_error" => "最近错误：",

    // ── GUI 运行日志面板 ──
    "gui.log.title" => "运行日志",

    // ── GUI 卸载对话框 ──
    "gui.uninstall.msix" => "卸载 MiDrop Ext (MSIX)",
    "gui.uninstall.product" => "卸载小米电脑管家 / 小米互联",

    // ── GUI 文件选择对话框 ──
    "gui.browse.title" => "选择安装包",
    "gui.browse.filter" => "安装包",

    // ── TUI ──
    "tui.shortcuts.global" => "退出",
    "tui.shortcuts.tab" => "切换面板",
    "tui.shortcuts.patches" => "q:退出  Tab:切换面板  ↑↓:选择补丁  ←→:选择按钮  Enter:执行  R:刷新  m:切换预设  c:自定义机型",
    "tui.shortcuts.install" => "q:退出  Tab:切换面板  Enter:查看安装指引",
    "tui.shortcuts.uninstall" => "q:退出  Tab:切换面板  ↑↓:选择  Enter:执行",
    "tui.shortcuts.log" => "q:退出  Tab:切换面板  ↑↓/PgUp/PgDn/Home/End:滚动日志",
    "tui.shortcuts.input" => "输入机型代号…  Enter:确认  Esc:取消  Backspace:退格",
    "tui.shortcuts.scroll" => "滚动",
    "tui.shortcuts.confirm" => "按 y 确认  ·  按 n / Esc 取消",
    "tui.shortcuts.input.confirm" => "Enter 确认 · Esc 取消",
    "tui.shortcuts.model.hint" => "m—切换预设  c—自定义",
    "tui.shortcuts.refresh" => "R — 刷新状态",

    // ── TUI 标签 ──
    "tui.model.custom" => "(自定义)",
    "tui.model.preset" => "机型",
    "tui.log.start" => "▶ 开始：{label}",
    "tui.log.done" => "✓ 完成：{label}",
    "tui.log.failed" => "{label} 失败：{error}",
    "tui.log.cancelled" => "已取消操作",
    "tui.log.error" => "❌ 错误：{error}",

    // ── TUI 安装面板 ──
    "tui.install.hint.focused" => "按 Enter — 查看安装指引（请在命令行中使用 install 子命令）",
    "tui.install.hint.unfocused" => "切换到本面板后按 Enter 查看安装指引",
    "tui.install.hint.cmd1" => "或：MiPCM_CLI install --installer <路径>",
    "tui.install.hint.cmd2" => "或：MiPCM_CLI install --url <下载地址>",

    // ── TUI 卸载提示 ──
    "tui.uninstall.key.hint" => "  Enter — 执行  ",

    // ── TUI 输入对话框 ──
    "tui.input.device" => "设备伪装",
    "tui.input.smbios" => "SMBIOS 伪装",
    "tui.input.title" => " 自定义机型 — {target_label} ",

    // ── TUI 状态 ──
    "tui.status.loading" => "（正在加载…）",
    "tui.confirm.title" => " ⚠ 确认卸载 ",

    // ── TUI 机型日志 ──
    "tui.log.model.set" => "📱 自定义设备机型已设置：{model}",
    "tui.log.smbios.set" => "📱 自定义 SMBIOS 机型已设置：{model}",
    "tui.log.model.switch" => "📱 设备伪装机型切换为：{code} [{name}]",
    "tui.log.smbios.switch" => "📱 SMBIOS 机型切换为：{code} [{name}]",

    // ── TUI 操作标签 ──
    "tui.op.locale.apply" => "地区伪装 · 应用",
    "tui.op.locale.revert" => "地区伪装 · 还原",
    "tui.op.camera.apply" => "摄像头弹窗抑制 · 应用",
    "tui.op.camera.revert" => "摄像头弹窗抑制 · 还原",
    "tui.op.audio.wifi" => "音频流转 · WiFi",
    "tui.op.audio.lan" => "音频流转 · LAN",
    "tui.op.audio.revert" => "音频流转 · 还原",
    "tui.op.device.apply" => "设备伪装 · 应用({model})",
    "tui.op.device.revert" => "设备伪装 · 还原",
    "tui.op.smbios.apply" => "SMBIOS · 应用({model})",
    "tui.op.smbios.revert" => "SMBIOS · 还原",
    "tui.op.uninstall.msix" => "卸载 MiDrop Ext (MSIX)",
    "tui.op.uninstall.product" => "卸载小米电脑管家 / 小米互联",

    // ── 卸载模块 ──
    "uninstall.start.manager" => "开始卸载小米电脑管家：{root}",
    "uninstall.start.continuity" => "开始卸载小米互联 / PcContinuity：{root}",
    "uninstall.running.uninstaller" => "  正在运行卸载程序：{exe}",
    "uninstall.remover.not.removed" => "  ⚠ 卸载程序未删除自身，卸载可能未完成：{exe}",
    "uninstall.main.done" => "  ✓ 主程序卸载完成",
    "uninstall.removing.services" => "  正在移除服务…",
    "uninstall.service.removed" => "    ✓ 已删除服务：{name}",
    "uninstall.service.failed" => "    ⚠ 删除服务 {name} 失败：{error}",
    "uninstall.cleaning.files" => "  正在清理文件…",
    "uninstall.done.manager" => "✓ 小米电脑管家卸载完成",
    "uninstall.done.continuity" => "✓ 小米互联 / PcContinuity 卸载完成",
    "uninstall.sub.not.installed" => "  - {label} 未安装，跳过",
    "uninstall.sub.uninstalling" => "  正在卸载 {label}：{exe}",
    "uninstall.sub.done" => "    ✓ {label} 卸载完成",
    "uninstall.sub.not.removed" => "    ⚠ {label} 卸载程序未删除自身，卸载可能未完成",
    "uninstall.sub.failed" => "    ⚠ 卸载 {label} 失败：{error}",
    "uninstall.sub.no.uninstaller" => "    ⚠ {label} 目录存在但未找到 uninstall.exe",
    "uninstall.sub.no.version" => "    ⚠ {label} 目录存在但无版本子目录",
    "uninstall.clean.programdata" => "    ✓ 已删除 C:\\ProgramData\\MI",
    "uninstall.clean.programdata.failed" => "    ⚠ 清理 C:\\ProgramData\\MI 失败：{error}",
    "uninstall.clean.dir" => "    ✓ 已删除 {path}",
    "uninstall.clean.dir.failed" => "    ⚠ 清理 {path} 失败：{error}",
    "uninstall.desc.manager" => "将卸载 小米电脑管家\n\n安装目录：{root}\n包含：主程序、AIService、MiService\n将删除所有相关服务与临时文件\n\n此操作不可逆！",
    "uninstall.desc.continuity" => "将卸载 小米互联 / PcContinuity\n\n安装目录：{root}\n将删除所有相关服务与临时文件\n\n此操作不可逆！",
    "uninstall.desc.coexistence" => "同时检测到小米电脑管家和小米互联。\n当前不支持同时安装，请逐一卸载。",
    "uninstall.desc.none" => "未检测到已安装的小米电脑管家或小米互联",
    "uninstall.no.msix" => "未检测到 MiDrop Ext MSIX 包",
    "uninstall.msix.done" => "✓ 已卸载 MiDrop Ext MSIX 包：{pkg}",
    "uninstall.explorer.restarted" => "✓ 已重启资源管理器",

    // ── Ops 模块 ──
    "ops.restart.hint" => "提示：补丁已完成，请手动重新启动小米电脑管家使其生效。",
});

tr_table!(tr_en, {
    // ── 通用 ──
    "app.title" => "MiPCM Patch",
    "btn.apply" => "Apply",
    "btn.revert" => "Revert",
    "btn.refresh" => "Refresh",
    "btn.cancel" => "Cancel",
    "btn.wifi" => "WiFi",
    "btn.lan" => "LAN",
    "btn.diagnose" => "Diagnose",
    "btn.fix" => "Fix",
    "btn.select" => "Select",
    "btn.start-install" => "Start Install",
    "btn.confirm" => "Confirm Uninstall",
    "btn.clear-log" => "Clear Log",
    "btn.browse" => "Browse…",
    "hint.custom" => "Custom",
    "hint.model.code" => "Model Code",
    "hint.custom.confirmed" => "Custom model set",
    "hint.custom.tui" => "c—Custom",
    "hint.preset.tui" => "m—Cycle Preset",
    "hint.uninstall.msix" => "Detect and remove MiDrop Ext MSIX package, then restart Explorer",
    "hint.uninstall.product" => "Complete uninstall: main program, sub-products, services, temp files (irreversible!)",
    "hint.uninstall.warn" => "Warning: uninstall is irreversible!",
    "hint.log.empty" => "Operation log will appear here…",
    "hint.install.tui" => "💡 Install via command line: MiPCM_CLI install",

    // ── 面板/章节标题 ──
    "section.status" => "Install Status",
    "section.patches" => "Patch Operations",
    "section.uninstall" => "Uninstall",
    "section.install" => "Install",
    "section.log" => "Log",
    "section.confirm.uninstall" => "Confirm Uninstall",
    "section.input.model" => " Custom Model — {target_label} ",
    "section.status.full" => "Install Status",

    // ── 补丁标签 ──
    "patch.locale" => "Locale Spoof",
    "patch.camera" => "Camera Toast",
    "patch.audio" => "Audio Stream",
    "patch.device" => "Device Spoof",
    "patch.smbios" => "Lyra SMBIOS (Experimental)",
    "patch.dual-nic" => "Lyra Dual NIC (Experimental)",
    "patch.locale.detail" => "Locale Spoof (micont_rtm.dll)",
    "patch.camera.detail" => "Camera Toast Suppression (PcControlCenter.dll)",
    "patch.audio.detail" => "Audio Stream Broadcast Mode",
    "patch.device.detail" => "Device Spoof (msimg32.dll)",
    "patch.smbios.detail" => "[Experimental] SMBIOS Device Identity",

    // ── CLI 命令 ──
    "cli.about" => "MiPCManager Enhancement Patch Tool",
    "cli.cmd.status" => "View installation info and patch status",
    "cli.cmd.locale" => "Locale spoof (micont_rtm.dll)",
    "cli.cmd.camera" => "Suppress camera toast (PcControlCenter.dll)",
    "cli.cmd.audio" => "MiPCAudio broadcast mode (wireless/wired, unified identity, dual-NIC fix)",
    "cli.cmd.device" => "Device spoof (deploy msimg32.dll + registry model)",
    "cli.cmd.smbios" => "[Experimental] SMBIOS device identity — Lyra special adaptation (micont_rtm.dll IAT Hook)",
    "cli.cmd.install" => "Install MiPCManager / MiContinuity (auto-detect product type)",
    "cli.cmd.uninstall" => "Uninstall MiDrop Ext MSIX or MiPCManager / MiContinuity",
    "cli.cmd.apply" => "Apply patch",
    "cli.cmd.revert" => "Revert patch",
    "cli.cmd.audio.apply" => "Switch broadcast mode",
    "cli.cmd.audio.revert" => "Revert audio stream patch",
    "cli.cmd.audio.experimental" => "[Experimental] Dual-NIC same-subnet audio fix: diagnose and align Wi-Fi routing with broadcast mode",
    "cli.cmd.device.apply" => "Apply device spoof",
    "cli.cmd.device.revert" => "Revert device spoof",
    "cli.cmd.smbios.apply" => "Apply SMBIOS device identity",
    "cli.cmd.smbios.revert" => "Revert SMBIOS device identity",
    "cli.cmd.msix" => "Uninstall MiDrop Ext MSIX package (restart Explorer afterwards)",
    "cli.cmd.product" => "Complete uninstall of MiPCManager / MiContinuity (main + sub-products + services + cleanup, irreversible)",

    // ── CLI 参数帮助 ──
    "cli.arg.dll" => "Specify target DLL path (auto-detect install dir by default)",
    "cli.arg.no_kill" => "Don't auto-close related processes",
    "cli.arg.region" => "Spoof region code (default CN)",
    "cli.arg.no_registry" => "Don't modify registry",
    "cli.arg.mode" => "Broadcast medium: wifi (wireless, default) or lan (wired)",
    "cli.arg.dir" => "Specify version directory (auto-detect by default)",
    "cli.arg.no_wifi_route" => "Don't auto-manage Wi-Fi local-subnet route (used in wireless mode for dual-NIC same-subnet fix)",
    "cli.arg.dry_run" => "Diagnose only, don't execute fix",
    "cli.arg.model" => "Model code (default TM2424)",
    "cli.arg.installer" => "Explicitly specify .exe installer",
    "cli.arg.url" => "Download from HTTP(S) URL",

    // ── CLI 运行时消息 ──
    "cli.error" => "Error: {error}",
    "cli.confirm.uninstall" => "Confirm uninstall? Enter y to continue: ",
    "cli.cancelled.uninstall" => "Uninstall cancelled.",
    "cli.cancelled.install" => "Install cancelled.",
    "cli.downloading" => "Downloading installer to {path}",
    "cli.found.installer" => "Found {kind} installer: {path}",
    "cli.multiple.installers" => "Found multiple installers:",
    "cli.choose.installer" => "Choose installer: ",
    "cli.invalid.choice" => "Invalid installer selection",
    "cli.no.installer.found" => "No installer found in Patcher directory (`*_XiaomiPCManager_*.exe` or `小米互联*.exe`).",
    "cli.cancel.option" => "Cancel",
    "cli.choose.option" => "Choose: ",
    "cli.menu.download_url" => "Enter download URL",
    "cli.menu.local_file" => "Specify local .exe installer",
    "cli.prompt.url" => "Enter HTTP(S) download URL: ",
    "cli.prompt.path" => "Enter .exe installer path: ",
    "cli.invalid.selection" => "Invalid selection",

    // ── 安装包分类 ──
    "install.kind.manager" => "MiPCManager (XiaomiPCManager)",
    "install.kind.continuity" => "MiContinuity / Interconnection (PcContinuity)",

    // ── 安装 GUI ──
    "install.title" => "Install MiPCManager / MiContinuity",
    "install.download.placeholder" => "HTTP(S) download URL…",
    "install.file.placeholder" => "Choose local .exe installer…",
    "install.download.and.install" => "Download & Install",

    // ── GUI 下载 ──
    "gui.downloading.start" => "Downloading: {url}",

    // ── GUI 安装操作标签 ──
    "gui.op.locale.apply" => "Locale Spoof",
    "gui.op.locale.revert" => "Locale Spoof · Revert",
    "gui.op.device.apply" => "Device Spoof({model})",
    "gui.op.device.revert" => "Device Spoof · Revert",
    "gui.op.camera.apply" => "Camera Toast",
    "gui.op.camera.revert" => "Camera Toast · Revert",
    "gui.op.audio.wifi" => "Audio · WiFi",
    "gui.op.audio.lan" => "Audio · LAN",
    "gui.op.audio.revert" => "Audio · Revert",
    "gui.op.dualnic.diagnose" => "Dual NIC · Diagnose",
    "gui.op.dualnic.fix" => "Dual NIC · Fix",
    "gui.op.smbios.apply" => "SMBIOS({model})",
    "gui.op.smbios.revert" => "SMBIOS · Revert",
    "gui.op.uninstall.msix" => "Uninstall MSIX",
    "gui.op.uninstall.product" => "Uninstall Product",
    "gui.op.install" => "Install({path})",

    // ── GUI 警告 ──
    "gui.warn.no_full" => "Full XiaomiPCManager not detected\nCamera / Audio / Device spoof unavailable.",
    "gui.warn.recent_error" => "Recent error: ",

    // ── GUI 运行日志面板 ──
    "gui.log.title" => "Operation Log",

    // ── GUI 卸载对话框 ──
    "gui.uninstall.msix" => "Uninstall MiDrop Ext (MSIX)",
    "gui.uninstall.product" => "Uninstall MiPCManager / MiContinuity",

    // ── GUI 文件选择对话框 ──
    "gui.browse.title" => "Choose Installer",
    "gui.browse.filter" => "Installer",

    // ── TUI ──
    "tui.shortcuts.global" => "Quit",
    "tui.shortcuts.tab" => "Switch Panel",
    "tui.shortcuts.patches" => "q:Quit  Tab:Panel  Up/Down:Select  Left/Right:Button  Enter:Run  R:Refresh  m:Preset  c:Custom",
    "tui.shortcuts.install" => "q:Quit  Tab:Panel  Enter:Install Guide",
    "tui.shortcuts.uninstall" => "q:Quit  Tab:Panel  Up/Down:Select  Enter:Run",
    "tui.shortcuts.log" => "q:Quit  Tab:Panel  Up/Down/PgUp/PgDn/Home/End:Scroll",
    "tui.shortcuts.input" => "Enter model code…  Enter:Confirm  Esc:Cancel  Backspace:Delete",
    "tui.shortcuts.scroll" => "Scroll",
    "tui.shortcuts.confirm" => "Press y to confirm  ·  Press n / Esc to cancel",
    "tui.shortcuts.input.confirm" => "Enter Confirm · Esc Cancel",
    "tui.shortcuts.model.hint" => "m—Cycle Preset  c—Custom",
    "tui.shortcuts.refresh" => "R — Refresh",

    // ── TUI 标签 ──
    "tui.model.custom" => "(Custom)",
    "tui.model.preset" => "Model",
    "tui.log.start" => "▶ Start: {label}",
    "tui.log.done" => "✓ Done: {label}",
    "tui.log.failed" => "{label} failed: {error}",
    "tui.log.cancelled" => "Operation cancelled",
    "tui.log.error" => "❌ Error: {error}",

    // ── TUI 安装面板 ──
    "tui.install.hint.focused" => "Press Enter — view install guide (use install subcommand in CLI)",
    "tui.install.hint.unfocused" => "Switch to this panel and press Enter to view install guide",
    "tui.install.hint.cmd1" => "   or: MiPCM_CLI install --installer <path>",
    "tui.install.hint.cmd2" => "   or: MiPCM_CLI install --url <url>",

    // ── TUI 卸载提示 ──
    "tui.uninstall.key.hint" => "  Enter — Run  ",

    // ── TUI 输入对话框 ──
    "tui.input.device" => "Device Spoof",
    "tui.input.smbios" => "SMBIOS Spoof",
    "tui.input.title" => " Custom Model — {target_label} ",

    // ── TUI 状态 ──
    "tui.status.loading" => "(Loading…)",
    "tui.confirm.title" => " ⚠ Confirm Uninstall ",

    // ── TUI 机型日志 ──
    "tui.log.model.set" => "📱 Custom device model set: {model}",
    "tui.log.smbios.set" => "📱 Custom SMBIOS model set: {model}",
    "tui.log.model.switch" => "📱 Device model switched to: {code} [{name}]",
    "tui.log.smbios.switch" => "📱 SMBIOS model switched to: {code} [{name}]",

    // ── TUI 操作标签 ──
    "tui.op.locale.apply" => "Locale Spoof · Apply",
    "tui.op.locale.revert" => "Locale Spoof · Revert",
    "tui.op.camera.apply" => "Camera Toast · Apply",
    "tui.op.camera.revert" => "Camera Toast · Revert",
    "tui.op.audio.wifi" => "Audio Stream · WiFi",
    "tui.op.audio.lan" => "Audio Stream · LAN",
    "tui.op.audio.revert" => "Audio Stream · Revert",
    "tui.op.device.apply" => "Device Spoof · Apply({model})",
    "tui.op.device.revert" => "Device Spoof · Revert",
    "tui.op.smbios.apply" => "SMBIOS · Apply({model})",
    "tui.op.smbios.revert" => "SMBIOS · Revert",
    "tui.op.uninstall.msix" => "Uninstall MiDrop Ext (MSIX)",
    "tui.op.uninstall.product" => "Uninstall MiPCManager / MiContinuity",

    // ── 卸载模块 ──
    "uninstall.start.manager" => "Starting uninstall of MiPCManager: {root}",
    "uninstall.start.continuity" => "Starting uninstall of MiContinuity / PcContinuity: {root}",
    "uninstall.running.uninstaller" => "  Running uninstaller: {exe}",
    "uninstall.remover.not.removed" => "  ⚠ Uninstaller did not delete itself, uninstall may be incomplete: {exe}",
    "uninstall.main.done" => "  ✓ Main program uninstall complete",
    "uninstall.removing.services" => "  Removing services…",
    "uninstall.service.removed" => "    ✓ Service removed: {name}",
    "uninstall.service.failed" => "    ⚠ Failed to remove service {name}: {error}",
    "uninstall.cleaning.files" => "  Cleaning files…",
    "uninstall.done.manager" => "✓ MiPCManager uninstall complete",
    "uninstall.done.continuity" => "✓ MiContinuity / PcContinuity uninstall complete",
    "uninstall.sub.not.installed" => "  - {label} not installed, skipped",
    "uninstall.sub.uninstalling" => "  Uninstalling {label}: {exe}",
    "uninstall.sub.done" => "    ✓ {label} uninstall complete",
    "uninstall.sub.not.removed" => "    ⚠ {label} uninstaller did not delete itself, uninstall may be incomplete",
    "uninstall.sub.failed" => "    ⚠ {label} uninstall failed: {error}",
    "uninstall.sub.no.uninstaller" => "    ⚠ {label} directory exists but uninstall.exe not found",
    "uninstall.sub.no.version" => "    ⚠ {label} directory exists but no version subdirectory",
    "uninstall.clean.programdata" => "    ✓ Removed C:\\ProgramData\\MI",
    "uninstall.clean.programdata.failed" => "    ⚠ Failed to clean C:\\ProgramData\\MI: {error}",
    "uninstall.clean.dir" => "    ✓ Removed {path}",
    "uninstall.clean.dir.failed" => "    ⚠ Failed to clean {path}: {error}",
    "uninstall.desc.manager" => "Will uninstall MiPCManager\n\nInstall dir: {root}\nIncludes: main app, AIService, MiService\nWill remove all services and temp files\n\nThis is irreversible!",
    "uninstall.desc.continuity" => "Will uninstall MiContinuity / PcContinuity\n\nInstall dir: {root}\nWill remove all services and temp files\n\nThis is irreversible!",
    "uninstall.desc.coexistence" => "Both MiPCManager and MiContinuity detected.\nCurrently does not support co-installation. Please uninstall one first.",
    "uninstall.desc.none" => "No installed MiPCManager or MiContinuity detected",
    "uninstall.no.msix" => "MiDrop Ext MSIX package not detected",
    "uninstall.msix.done" => "✓ Uninstalled MiDrop Ext MSIX: {pkg}",
    "uninstall.explorer.restarted" => "✓ Explorer restarted",

    // ── Ops 模块 ──
    "ops.restart.hint" => "Hint: Patches applied. Please restart MiPCManager manually for changes to take effect.",
});
