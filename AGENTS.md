# AGENTS.md — 项目范式与编码规约

> 本文档面向 Coding Agent（及人类贡献者），定义 MiPCManager_Patch 项目的目录结构、编码范式、命名约定与设计原则。所有改动需遵守本文。

---

## 1. 项目定位

为「小米电脑管家 (XiaomiPCManager) / 小米互联 (PcContinuity)」提供**功能增强补丁**的 Windows 工具。

- **语言**：Rust（edition 2024）
- **平台**：Windows only（大量 `#[cfg(windows)]`）
- **产物**：单一 CLI/TUI 二进制 (`mipcm_patch`) + 独立 GUI 二进制 (`mipcm_gui`)

---

## 2. 目录结构

```
src/
├── lib.rs                    # 库根：只做模块声明，不放逻辑
├── main.rs                   # 统一入口：有 CLI 参数→dispatch，无参数→启动 TUI
├── ops.rs                    # 高层操作：PatchOp 组合子编排，面向前端
├── elevate.rs                # 管理员提权（release manifest + 运行时兜底）
│
├── infra/                    # 通用基础设施（本项目最底层，零业务语义）
│   ├── mod.rs
│   ├── pe.rs                 # PE32+ 解析（读取节表、RVA↔偏移、追加节、校验和）
│   ├── bytes.rs              # 字节搜索/替换工具（find_bytes, locate_single_byte）
│   └── registry.rs           # 注册表读写（set/delete/get HKCU string）
│
├── install/                  # 安装目录探测、进程管理、文件备份/原子写
│   ├── mod.rs
│   └── pc_manager_installer.rs
│
├── patches/                  # 各功能补丁（每个模块：对外 apply / revert / current_state）
│   ├── mod.rs
│   ├── locale.rs             # 地区伪装
│   ├── camera.rs             # 摄像头 .NET 方法体注入
│   │   └── dotnet/           # .NET 专用子模块（metadata / method_body）
│   ├── audio.rs              # MiPCAudio 音频流转
│   └── device.rs             # 设备伪装（msimg32.dll + 注册表）
│
├── experimental/             # 实验性功能（不参与本次重构的模块化修改）
│   ├── mod.rs
│   ├── audio_dual_nic.rs
│   ├── smbios.rs
│   ├── smbios_spoof.rs
│   └── x64_trampoline.rs
│
└── ui/                       # 前端（不导出到 lib，仅二进制入口引用）
    ├── mod.rs
    ├── gui/main.rs           # egui + eframe 图形界面
    └── tui/                  # ratatui + crossterm 终端界面
        ├── mod.rs
        ├── app.rs
        ├── widgets.rs
        └── theme.rs
```

### 模块依赖方向（严格单向）

```
ui (gui / tui)
    └→ ops
        └→ patches  ─┐
          └→ install ─┼→ infra
            └→ infra ─┘
```

- `infra` 不依赖任何业务模块
- `patches` 依赖 `infra` + `install`
- `ops` 依赖 `patches` + `install`
- `ui` 依赖 `ops`（及必要的 `patches` 常量）

---

## 3. 核心设计原则

### 3.1 组合子优先于过程式

每个补丁操作遵循同一流水线，**必须用 `PatchOp` 组合子表达**，禁止手写重复流程：

```rust
// ✓ 正确：声明式
run_patch(PatchOp {
    procs: PROC_CAMERA,
    require_killed: false,
    action: || patches::camera::apply(&path),
    on_success: vec![format!("✓ 已应用：{}", path.display())],
}, no_kill)

// ✗ 错误：过程式手写流程
let mut log = Vec::new();
close_apps(PROC_CAMERA, no_kill, &mut log);
let outcome = retry_patch_after_access_denied(PROC_CAMERA, &mut log, || ...)?;
// ...
```

### 3.2 泛型约束见类型签名

```rust
pub struct PatchOp<A, S>
where
    A: FnOnce() -> Result<S>,
    S: Into<Vec<String>>,
{ ... }

pub fn run_patch<A, S>(op: PatchOp<A, S>, no_kill: bool) -> Result<Vec<String>>
where
    A: FnOnce() -> Result<S>,
    S: Into<Vec<String>>,
{ ... }
```

### 3.3 模块内聚：一个功能一个模块

| 模块 | 对外 API | 不应暴露 |
|---|---|---|
| `patches/locale` | `apply()`, `revert()`, `TARGET_DLL`, `PatchOutcome` | 内部字节偏移逻辑 |
| `patches/camera` | `apply()`, `revert()`, `TARGET_DLL` | dotnet 子模块细节 |
| `patches/audio` | `apply()`, `revert()`, `current_state()`, `BroadcastMode` | 特征码常量 |
| `patches/device` | `apply()`, `revert()`, `deploy_proxy()`, `current_state()` | 内嵌 DLL 字节 |

### 3.4 工具函数统一到 infra

- **字节搜索** → `infra::bytes`（不允许各模块自行实现 `find` / `locate`）
- **注册表** → `infra::registry`（不允许各模块自己调 `winreg`）
- **PE 解析** → `infra::pe`（所有 PE 操作经此模块）

---

## 4. 编码约定

### 4.1 命名

| 元素 | 约定 | 示例 |
|---|---|---|
| 模块文件 | snake_case | `audio_wifi_route.rs` → 合并后为 `patches/audio.rs` |
| 公开常量 | SCREAMING_SNAKE_CASE | `PROC_MIPCM_ALL`, `TARGET_DLL`, `DEFAULT_MODEL` |
| 补丁结果枚举 | `PatchOutcome` | `Patched`, `AlreadyPatched` |
| 高层函数 | `apply_<feature>` / `revert_<feature>` | `apply_locale`, `revert_camera` |
| 进程常量 | `PROC_<FEATURE>` | `PROC_LOCALE`, `PROC_AUDIO` |

### 4.2 错误处理

- 统一使用 `anyhow::Result<T>`（库内不定义自定义错误类型，除非有可编程匹配需求）
- 补丁前备份：`install::ensure_backup(path)?`
- 补丁写回：`install::write_file_atomic(path, &data)?`
- 文件占用重试：`retry_patch_after_access_denied(procs, log, action)?`

### 4.3 Windows 条件编译

```rust
#[cfg(windows)]
pub fn kill_by_names(names: &[&str]) -> usize { ... }

#[cfg(not(windows))]
pub fn kill_by_names(_names: &[&str]) -> usize { 0 }
```

### 4.4 日志模型

所有 `ops` 函数返回 `Result<Vec<String>>`，每行一条日志。前端（CLI/GUI/TUI）负责呈现：

```rust
// CLI
for line in ops::apply_locale(None, "CN", true, false)? {
    println!("{line}");
}

// GUI / TUI
for line in result? {
    self.log.push(line);
}
```

### 4.5 幂等性

所有补丁必须实现：
- 重复 `apply` → 识别已打补丁 → 返回 `AlreadyPatched` / 跳过
- `revert` → 从 `.orig.bak` 恢复 → 不存在则报错

---

## 5. 依赖与编译

### 5.1 运行时依赖

```toml
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }

[target.'cfg(windows)'.dependencies]
winreg = "0.56"
sysinfo = "0.39"
windows-sys = "0.61"     # Win32 API（提权、进程注入、SMBIOS）
eframe = "0.35"           # GUI
egui = "0.35"
ico = "0.5"
rfd = "0.17"
ratatui = "0.29"          # TUI
crossterm = "0.28"
tokio = { version = "1", features = ["rt", "macros", "sync"] }

[build-dependencies]
embed-resource = "3.0.11"  # 嵌入 .manifest（管理员权限）

[profile.release]
opt-level = "z"            # 优化体积
lto = true
strip = true
panic = "abort"
```

### 5.2 二进制入口

```toml
[[bin]]
name = "mipcm_patch"
path = "src/main.rs"

[[bin]]
name = "mipcm_gui"
path = "src/ui/gui/main.rs"
```

### 5.3 内嵌资源

| 文件 | 方式 | 使用者 |
|---|---|---|
| `assets/MiPCManager.ico` | GUI: `ico` crate 解析 | `ui/gui/main.rs` |
| `patches/dlls/msimg32.dll` | `include_bytes!` | `patches/device.rs` |
| `resources/*.manifest` | `embed-resource` build.rs | 所有二进制产物 |

---

## 6. 设计决策记录

### ADR-001：PatchOp 组合子而非宏

**选择**：泛型结构体 `PatchOp<A, S>` + 函数 `run_patch`

**原因**：
- 宏会模糊类型边界，IDE 支持差
- 泛型 + FnOnce 给 Rust 编译器足够信息做内联优化
- 每个操作的特有日志通过 `S: Into<Vec<String>>` 注入

### ADR-002：ratatui + crossterm 做 TUI

**选择**：ratatui 0.29 + crossterm 0.28（非 termion）

**原因**：
- crossterm 纯 Rust 跨平台，Windows 原生支持好
- ratatui 是 Rust TUI 生态事实标准
- 事件流使用 `crossterm::event::EventStream` + tokio

### ADR-003：infra 作为最底层

**选择**：将 PE 解析、字节搜索、注册表从业务模块中提升到 `infra/`

**原因**：
- `pe.rs` 被 camera 和 smbios_spoof 共用，不应藏在 camera_toast 子模块
- 字节搜索在 3 处重复实现，统一后可复用
- 注册表操作在 2 处重复，统一后减少 winreg 直接依赖

### ADR-004：patches 按功能而非按技术分类

**选择**：`patches/locale` `patches/camera` `patches/audio` `patches/device`

**原因**：每个补丁是独立功能，用户按功能理解；技术细节（.NET、PE）内聚在 infra 或子模块中

---

## 7. 常见任务指南

### 新增一个补丁

1. 在 `patches/` 下新建模块，实现 `apply()` 和 `revert()`
2. 在 `patches/mod.rs` 声明模块并重导出公共符号
3. 在 `ops.rs` 中添加：
   - 进程常量 `PROC_*`
   - `apply_*()` / `revert_*()` 函数（调用 `run_patch`）
4. 在 `main.rs` CLI 子命令中添加对应分支
5. 在 UI（gui + tui）中添加对应操作入口

### 新增一个 CLI 子命令

1. 在 `main.rs` 的 `Command` 枚举中添加变体
2. 在 `run()` 函数中添加 match 分支
3. 若需交互菜单，在 TUI 的 `app.rs` 中添加菜单项

### 编译检查前

```text
cargo check --lib       # 库编译
cargo check --bin mipcm_patch   # CLI/TUI 入口
cargo check --bin mipcm_gui     # GUI 入口
cargo test
cargo clippy -- -D warnings
```

---

## 8. 禁止事项

- **禁止**在 `patches/` 中直接调用 `winreg`（走 `infra::registry`）
- **禁止**在业务模块中实现私有 `find()` / `find_bytes()`（走 `infra::bytes`）
- **禁止**在 `ops.rs` 中手写关进程→备份→补丁→重试流程（走 `run_patch`）
- **禁止**在 `lib.rs` 中放置任何逻辑代码（仅 `pub mod` 声明）
- **禁止**修改 `experimental/` 中文件的功能逻辑（仅允许更新导入路径）
- **禁止**在同一 PR 中混合功能变更与重构（重构先行，功能后行）
