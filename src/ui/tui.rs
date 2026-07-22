//! ratatui TUI 前端：终端可视化界面，展示安装状态、补丁操作、日志输出。
//!
//! 无参数启动时自动进入 crossterm raw mode + alternate screen，退出时完整还原终端。
//! 补丁操作通过 tokio 异步执行，不阻塞界面响应；日志实时追加到日志面板。

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::{
    ops,
    patches::device,
};

// ── TUI entry point ────────────────────────────────────────────────

/// 启动 ratatui TUI 界面。在 `main.rs` 中无命令行参数时调用。
pub fn run() -> Result<()> {
    // 初始化终端
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // 创建 tokio runtime 用于异步补丁操作
    let rt = tokio::runtime::Runtime::new()?;
    let (log_tx, log_rx) = mpsc::channel::<LogMessage>();

    let mut app = App::new(rt, log_tx);

    // ratatui terminal
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let result = app.run_loop(&mut terminal, log_rx);

    // 还原终端
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ── Log message type ───────────────────────────────────────────────

enum LogMessage {
    Line(String),
    Done,
    Error(String),
}

// ── Panel focus ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Patches,
    Install,
    Log,
}

// ── Patch selection ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchRow {
    Locale,
    Camera,
    Audio,
    Device,
    Smbios,
}

impl PatchRow {
    const ALL: &[PatchRow] = &[
        PatchRow::Locale,
        PatchRow::Camera,
        PatchRow::Audio,
        PatchRow::Device,
        PatchRow::Smbios,
    ];

    fn label(&self) -> &'static str {
        match self {
            PatchRow::Locale => "地区伪装 (micont_rtm.dll)",
            PatchRow::Camera => "摄像头弹窗抑制 (PcControlCenter.dll)",
            PatchRow::Audio => "音频流转广播模式",
            PatchRow::Device => "设备伪装 (msimg32.dll)",
            PatchRow::Smbios => "[实验性] SMBIOS 设备身份",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Apply,
    Revert,
    Wifi,
    Lan,
}

// ── Text input mode ───────────────────────────────────────────────

/// 当用户在设备/SMBIOS 补丁上按 `c` 时进入自定义机型输入模式。
struct InputMode {
    /// 输入目标：Device 或 Smbios。
    target: PatchRow,
    /// 当前输入缓冲区。
    buffer: String,
}

// ── App state ──────────────────────────────────────────────────────

struct App {
    // Terminal size tracking
    _prev_size: Option<(u16, u16)>,

    // Async runtime & channel
    rt: tokio::runtime::Runtime,
    log_tx: Sender<LogMessage>,

    // Focus
    focused_panel: Panel,
    selected_patch: usize,   // index into PatchRow::ALL
    selected_button: usize,  // index into current row's buttons

    // Model config for device / smbios
    device_model_idx: usize,
    smbios_model_idx: usize,
    custom_device_model: Option<String>,
    custom_smbios_model: Option<String>,

    // Text input mode for custom model
    input_mode: Option<InputMode>,

    // Status cache
    status_lines: Vec<String>,
    full_features: bool,

    // Log
    log: Vec<String>,
    log_scroll: usize,

    // Operation in progress
    op_running: bool,
    op_label: String,
}

impl App {
    fn new(rt: tokio::runtime::Runtime, log_tx: Sender<LogMessage>) -> Self {
        let mut app = Self {
            _prev_size: None,
            rt,
            log_tx,
            focused_panel: Panel::Patches,
            selected_patch: 0,
            selected_button: 0,
            device_model_idx: 0,
            smbios_model_idx: 0,
            custom_device_model: None,
            custom_smbios_model: None,
            input_mode: None,
            status_lines: Vec::new(),
            full_features: false,
            log: Vec::new(),
            log_scroll: 0,
            op_running: false,
            op_label: String::new(),
        };
        app.refresh_status();
        app
    }

    // ── Event loop ─────────────────────────────────────────────

    fn run_loop(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
        log_rx: Receiver<LogMessage>,
    ) -> Result<()> {
        let tick = Duration::from_millis(50);
        loop {
            // Drain log channel
            self.drain_log(&log_rx);

            // Render
            let _ = terminal.draw(|f| self.render(f));

            // Poll keyboard
            if event::poll(tick)?
                && let Event::Key(key) = event::read()?
            {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if !self.handle_key(key.code) {
                    return Ok(());
                }
            }
        }
    }

    fn drain_log(&mut self, rx: &Receiver<LogMessage>) {
        loop {
            match rx.try_recv() {
                Ok(LogMessage::Line(line)) => {
                    self.log.push(line);
                    // Auto-scroll to bottom
                    self.log_scroll = self.log.len().saturating_sub(1);
                }
                Ok(LogMessage::Done) => {
                    self.op_running = false;
                    self.op_label.clear();
                    self.refresh_status();
                }
                Ok(LogMessage::Error(e)) => {
                    self.log.push(format!("❌ 错误：{e}"));
                    self.op_running = false;
                    self.op_label.clear();
                    self.log_scroll = self.log.len().saturating_sub(1);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    // ── Keyboard handler ───────────────────────────────────────

    /// Returns `false` if the app should quit.
    fn handle_key(&mut self, code: KeyCode) -> bool {
        // Text input mode intercepts most keys
        if self.input_mode.is_some() {
            match code {
                KeyCode::Esc => {
                    self.input_mode = None;
                }
                KeyCode::Enter => {
                    let input = self.input_mode.take().unwrap();
                    let model = input.buffer.trim().to_string();
                    if !model.is_empty() {
                        match input.target {
                            PatchRow::Device => {
                                self.custom_device_model = Some(model.clone());
                                self.log.push(format!("📱 自定义设备机型已设置：{model}"));
                                self.log_scroll = self.log.len().saturating_sub(1);
                            }
                            PatchRow::Smbios => {
                                self.custom_smbios_model = Some(model.clone());
                                self.log.push(format!("📱 自定义 SMBIOS 机型已设置：{model}"));
                                self.log_scroll = self.log.len().saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ref mut input) = self.input_mode {
                        input.buffer.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut input) = self.input_mode {
                        input.buffer.push(c);
                    }
                }
                _ => {}
            }
            return true;
        }

        // Global keys (always active)
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return false,
            KeyCode::Esc => return false,
            KeyCode::Tab => {
                self.focused_panel = match self.focused_panel {
                    Panel::Patches => Panel::Install,
                    Panel::Install => Panel::Log,
                    Panel::Log => Panel::Patches,
                };
                self.selected_button = 0;
                return true;
            }
            KeyCode::Char('R') => {
                self.refresh_status();
                return true;
            }
            _ => {}
        }

        // Panel-specific keys
        if self.op_running {
            return true; // block input during operations
        }

        match self.focused_panel {
            Panel::Patches => self.handle_patches_key(code),
            Panel::Install => self.handle_install_key(code),
            Panel::Log => self.handle_log_key(code),
        }
        true
    }

    fn handle_patches_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => {
                if self.selected_patch > 0 {
                    self.selected_patch -= 1;
                    self.selected_button = 0;
                }
            }
            KeyCode::Down => {
                if self.selected_patch < PatchRow::ALL.len() - 1 {
                    self.selected_patch += 1;
                    self.selected_button = 0;
                }
            }
            KeyCode::Left => {
                if self.selected_button > 0 {
                    self.selected_button -= 1;
                }
            }
            KeyCode::Right => {
                let max = self.current_patch_button_count() - 1;
                if self.selected_button < max {
                    self.selected_button += 1;
                }
            }
            KeyCode::Enter => {
                self.activate_patch_button();
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.cycle_model();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                let patch = self.current_patch();
                if matches!(patch, PatchRow::Device | PatchRow::Smbios) {
                    self.input_mode = Some(InputMode {
                        target: patch,
                        buffer: String::new(),
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_install_key(&mut self, code: KeyCode) {
        if code == KeyCode::Enter {
            self.spawn_install();
        }
    }

    fn handle_log_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.log_scroll + 1 < self.log.len() {
                    self.log_scroll += 1;
                }
            }
            KeyCode::PageUp => {
                self.log_scroll = self.log_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.log_scroll = (self.log_scroll + 10).min(self.log.len().saturating_sub(1));
            }
            KeyCode::Home => {
                self.log_scroll = 0;
            }
            KeyCode::End => {
                self.log_scroll = self.log.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    // ── Patch button logic ─────────────────────────────────────

    fn current_patch(&self) -> PatchRow {
        PatchRow::ALL[self.selected_patch]
    }

    fn current_patch_button_count(&self) -> usize {
        match self.current_patch() {
            PatchRow::Audio => 3, // Wifi, Lan, Revert
            _ => 2,               // Apply, Revert
        }
    }

    fn current_button(&self) -> Option<Button> {
        match self.current_patch() {
            PatchRow::Audio => match self.selected_button {
                0 => Some(Button::Wifi),
                1 => Some(Button::Lan),
                2 => Some(Button::Revert),
                _ => None,
            },
            _ => match self.selected_button {
                0 => Some(Button::Apply),
                1 => Some(Button::Revert),
                _ => None,
            },
        }
    }

    fn activate_patch_button(&mut self) {
        let patch = self.current_patch();
        let button = self.current_button();
        match (patch, button) {
            (PatchRow::Locale, Some(Button::Apply)) => self.spawn_locale_apply(),
            (PatchRow::Locale, Some(Button::Revert)) => self.spawn_locale_revert(),
            (PatchRow::Camera, Some(Button::Apply)) => self.spawn_camera_apply(),
            (PatchRow::Camera, Some(Button::Revert)) => self.spawn_camera_revert(),
            (PatchRow::Audio, Some(Button::Wifi)) => self.spawn_audio_wifi(),
            (PatchRow::Audio, Some(Button::Lan)) => self.spawn_audio_lan(),
            (PatchRow::Audio, Some(Button::Revert)) => self.spawn_audio_revert(),
            (PatchRow::Device, Some(Button::Apply)) => self.spawn_device_apply(),
            (PatchRow::Device, Some(Button::Revert)) => self.spawn_device_revert(),
            (PatchRow::Smbios, Some(Button::Apply)) => self.spawn_smbios_apply(),
            (PatchRow::Smbios, Some(Button::Revert)) => self.spawn_smbios_revert(),
            _ => {}
        }
    }

    fn cycle_model(&mut self) {
        let presets_len = device::PRESETS.len();
        match self.current_patch() {
            PatchRow::Device => {
                self.custom_device_model = None; // clear custom when cycling presets
                self.device_model_idx = (self.device_model_idx + 1) % presets_len;
                self.log.push(format!(
                    "📱 设备伪装机型切换为：{} [{}]",
                    device::PRESETS[self.device_model_idx].code,
                    device::PRESETS[self.device_model_idx].name
                ));
                self.log_scroll = self.log.len().saturating_sub(1);
            }
            PatchRow::Smbios => {
                self.custom_smbios_model = None; // clear custom when cycling presets
                self.smbios_model_idx = (self.smbios_model_idx + 1) % presets_len;
                self.log.push(format!(
                    "📱 SMBIOS 机型切换为：{} [{}]",
                    device::PRESETS[self.smbios_model_idx].code,
                    device::PRESETS[self.smbios_model_idx].name
                ));
                self.log_scroll = self.log.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    fn current_device_model(&self) -> &str {
        self.custom_device_model
            .as_deref()
            .unwrap_or(device::PRESETS[self.device_model_idx].code)
    }

    fn current_smbios_model(&self) -> &str {
        self.custom_smbios_model
            .as_deref()
            .unwrap_or(device::PRESETS[self.smbios_model_idx].code)
    }

    // ── Status ─────────────────────────────────────────────────

    fn refresh_status(&mut self) {
        self.status_lines = ops::status_lines();
        self.full_features = ops::full_features_available();
    }

    // ── Async spawns ───────────────────────────────────────────

    fn spawn_op<F>(&mut self, label: &str, f: F)
    where
        F: FnOnce() -> Result<Vec<String>> + Send + 'static,
    {
        if self.op_running {
            return;
        }
        self.op_running = true;
        self.op_label = label.to_string();
        let tx = self.log_tx.clone();
        let label = label.to_string();
        self.rt.spawn(async move {
            let _ = tx.send(LogMessage::Line(format!("▶ 开始：{label}")));
            match f() {
                Ok(lines) => {
                    for line in lines {
                        let _ = tx.send(LogMessage::Line(line));
                    }
                    let _ = tx.send(LogMessage::Line(format!("✓ 完成：{label}")));
                }
                Err(e) => {
                    let _ = tx.send(LogMessage::Error(format!("{label} 失败：{e:#}")));
                }
            }
            let _ = tx.send(LogMessage::Done);
        });
    }

    fn spawn_locale_apply(&mut self) {
        self.spawn_op("地区伪装 · 应用", || ops::apply_locale(None, "CN", true, false));
    }

    fn spawn_locale_revert(&mut self) {
        self.spawn_op("地区伪装 · 还原", || ops::revert_locale(None, true, false));
    }

    fn spawn_camera_apply(&mut self) {
        self.spawn_op("摄像头弹窗抑制 · 应用", || ops::apply_camera(None, false));
    }

    fn spawn_camera_revert(&mut self) {
        self.spawn_op("摄像头弹窗抑制 · 还原", || ops::revert_camera(None, false));
    }

    fn spawn_audio_wifi(&mut self) {
        self.spawn_op("音频流转 · WiFi", || {
            ops::apply_audio(ops::BroadcastMode::Wireless, None, false)
        });
    }

    fn spawn_audio_lan(&mut self) {
        self.spawn_op("音频流转 · LAN", || {
            ops::apply_audio(ops::BroadcastMode::Wired, None, false)
        });
    }

    fn spawn_audio_revert(&mut self) {
        self.spawn_op("音频流转 · 还原", || ops::revert_audio(None, false));
    }

    fn spawn_device_apply(&mut self) {
        let model = self.current_device_model().to_string();
        self.spawn_op(&format!("设备伪装 · 应用({model})"), move || {
            ops::apply_device(&model, None, false)
        });
    }

    fn spawn_device_revert(&mut self) {
        self.spawn_op("设备伪装 · 还原", || ops::revert_device(None, false));
    }

    fn spawn_smbios_apply(&mut self) {
        let model = self.current_smbios_model().to_string();
        self.spawn_op(&format!("SMBIOS · 应用({model})"), move || {
            ops::apply_smbios(Some(&model), None, false)
        });
    }

    fn spawn_smbios_revert(&mut self) {
        self.spawn_op("SMBIOS · 还原", || ops::revert_smbios(None, false));
    }

    fn spawn_install(&mut self) {
        // 安装流程需要交互式选择安装包，不适合在 TUI 后台执行。
        // 在日志面板提示用户通过命令行 `MiPCM_CLI install` 执行。
        self.log.push("💡 安装请使用命令行：MiPCM_CLI install".to_string());
        self.log.push("   或：MiPCM_CLI install --installer <路径>".to_string());
        self.log.push("   或：MiPCM_CLI install --url <下载地址>".to_string());
        self.log_scroll = self.log.len().saturating_sub(1);
    }

    // ── Rendering ──────────────────────────────────────────────

    fn render(&self, f: &mut Frame) {
        let area = f.area();

        // Main vertical layout
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // title
                Constraint::Min(8),     // status + patches
                Constraint::Length(3),  // install
                Constraint::Min(4),     // log
                Constraint::Length(1),  // shortcuts
            ])
            .split(area);

        self.render_title(f, v[0]);
        self.render_main(f, v[1]);
        self.render_install(f, v[2]);
        self.render_log(f, v[3]);
        self.render_shortcuts(f, v[4]);

        // Text input overlay
        if let Some(ref input) = self.input_mode {
            self.render_input_overlay(f, input);
        }
    }

    fn render_title(&self, f: &mut Frame, area: Rect) {
        let title = if self.op_running {
            format!(" MiPCM Patch v{} — ⏳ {}…", env!("CARGO_PKG_VERSION"), self.op_label)
        } else {
            format!(" MiPCM Patch v{}", env!("CARGO_PKG_VERSION"))
        };
        let p = Paragraph::new(title)
            .style(Style::default().fg(Color::White).bg(Color::Rgb(255, 105, 0)))
            .alignment(Alignment::Left);
        f.render_widget(p, area);
    }

    fn render_main(&self, f: &mut Frame, area: Rect) {
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        self.render_status(f, h[0]);
        self.render_patches(f, h[1]);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let border_style = Style::default().fg(Color::Gray);
        let block = Block::default()
            .title(" 安装状态 ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        f.render_widget(block, area);

        let lines: Vec<Line> = if self.status_lines.is_empty() {
            vec![Line::from("（正在加载…）")]
        } else {
            self.status_lines
                .iter()
                .map(|s| Line::from(s.as_str()))
                .collect()
        };

        let text = Text::from(lines);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, inner);

        // Refresh hint
        let hint = Paragraph::new("R — 刷新状态")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Right);
        f.render_widget(hint, Rect::new(inner.x, inner.y + inner.height.saturating_sub(1), inner.width, 1));
    }

    fn render_patches(&self, f: &mut Frame, area: Rect) {
        let focused = self.focused_panel == Panel::Patches;
        let border_style = if focused {
            Style::default().fg(Color::Rgb(255, 105, 0))
        } else {
            Style::default().fg(Color::Gray)
        };
        let block = Block::default()
            .title(" 补丁操作 ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Build patch rows
        let mut lines: Vec<Line> = Vec::new();
        for (i, patch) in PatchRow::ALL.iter().enumerate() {
            let is_selected = focused && i == self.selected_patch;
            let highlight = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Rgb(255, 105, 0))
            } else {
                Style::default().fg(Color::White)
            };

            // Patch label
            let marker = if is_selected { "▶" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("{marker} {}", patch.label()),
                highlight,
            )));

            // Buttons row
            lines.push(self.render_patch_buttons(i, is_selected, highlight));

            // Model row for device / smbios
            match patch {
                PatchRow::Device => {
                    let model = self.current_device_model();
                    let model_text = if self.custom_device_model.is_some() {
                        format!("{} (自定义)", model)
                    } else {
                        let preset = &device::PRESETS[self.device_model_idx];
                        format!("{} ({})", model, preset.name)
                    };
                    lines.push(Line::from(vec![
                        Span::styled("     机型：", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            model_text,
                            if is_selected {
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::Cyan)
                            },
                        ),
                        Span::styled("  m—切换预设  c—自定义", Style::default().fg(Color::DarkGray)),
                    ]));
                }
                PatchRow::Smbios => {
                    let model = self.current_smbios_model();
                    let model_text = if self.custom_smbios_model.is_some() {
                        format!("{} (自定义)", model)
                    } else {
                        let preset = &device::PRESETS[self.smbios_model_idx];
                        format!("{} ({})", model, preset.name)
                    };
                    lines.push(Line::from(vec![
                        Span::styled("     机型：", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            model_text,
                            if is_selected {
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::Cyan)
                            },
                        ),
                        Span::styled("  m—切换预设  c—自定义", Style::default().fg(Color::DarkGray)),
                    ]));
                }
                _ => {}
            }

            // Spacer (except after last)
            if i < PatchRow::ALL.len() - 1 {
                lines.push(Line::from(""));
            }
        }

        let p = Paragraph::new(Text::from(lines));
        f.render_widget(p, inner);
    }

    fn render_patch_buttons(
        &self,
        patch_idx: usize,
        is_selected: bool,
        highlight: Style,
    ) -> Line<'static> {
        let patch = PatchRow::ALL[patch_idx];
        let buttons: &[(&str, Option<Button>)] = match patch {
            PatchRow::Audio => &[
                ("WiFi", Some(Button::Wifi)),
                ("LAN", Some(Button::Lan)),
                ("还原", Some(Button::Revert)),
            ],
            _ => &[
                ("应用", Some(Button::Apply)),
                ("还原", Some(Button::Revert)),
            ],
        };

        let mut spans: Vec<Span> = vec![Span::styled("     ", highlight)];

        for (bi, (label, _btn)) in buttons.iter().enumerate() {
            let btn_selected = is_selected && bi == self.selected_button;
            let btn_style = if btn_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(255, 180, 50))
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::Rgb(255, 180, 50))
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("[{}]", label), btn_style));
            spans.push(Span::styled(" ", highlight));
        }

        Line::from(spans)
    }

    fn render_install(&self, f: &mut Frame, area: Rect) {
        let focused = self.focused_panel == Panel::Install;
        let border_style = if focused {
            Style::default().fg(Color::Rgb(255, 105, 0))
        } else {
            Style::default().fg(Color::Gray)
        };
        let block = Block::default()
            .title(" 安装 ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        f.render_widget(block, area);

        let hint = if focused {
            "按 Enter — 查看安装指引（请在命令行中使用 install 子命令）"
        } else {
            "切换到本面板后按 Enter 查看安装指引"
        };

        let p = Paragraph::new(hint)
            .style(if focused {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            })
            .alignment(Alignment::Center);
        f.render_widget(p, inner);
    }

    fn render_log(&self, f: &mut Frame, area: Rect) {
        let focused = self.focused_panel == Panel::Log;
        let border_style = if focused {
            Style::default().fg(Color::Rgb(255, 105, 0))
        } else {
            Style::default().fg(Color::Gray)
        };
        let block = Block::default()
            .title(" 日志 ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Show visible portion of log
        let visible_height = inner.height as usize;
        let total = self.log.len();
        if total == 0 {
            let p = Paragraph::new("操作日志将显示在这里…")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(p, inner);
            return;
        }

        let start = if total <= visible_height {
            0
        } else {
            self.log_scroll
                .saturating_sub(visible_height.saturating_sub(1))
                .min(total.saturating_sub(visible_height))
        };
        let end = (start + visible_height).min(total);

        let items: Vec<ListItem> = self.log[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let style = if start + i == self.log_scroll && focused {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(255, 105, 0))
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(line.as_str(), style)))
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, inner);

        // Scroll indicator
        if total > visible_height {
            let pct = (self.log_scroll as f64 / total as f64 * 100.0) as usize;
            let indicator = format!("{pct}% — ↑↓/PgUp/PgDn/Home/End 滚动");
            let p = Paragraph::new(indicator)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Right);
            f.render_widget(
                p,
                Rect::new(
                    inner.x,
                    inner.y + inner.height.saturating_sub(1),
                    inner.width,
                    1,
                ),
            );
        }
    }

    fn render_shortcuts(&self, f: &mut Frame, area: Rect) {
        let keys = if self.input_mode.is_some() {
            "输入机型代号…  Enter:确认  Esc:取消  Backspace:退格"
        } else {
            match self.focused_panel {
                Panel::Patches => "q:退出  Tab:切换面板  ↑↓:选择补丁  ←→:选择按钮  Enter:执行  R:刷新  m:切换预设  c:自定义机型",
                Panel::Install => "q:退出  Tab:切换面板  Enter:查看安装指引",
                Panel::Log => "q:退出  Tab:切换面板  ↑↓/PgUp/PgDn/Home/End:滚动日志",
            }
        };

        let p = Paragraph::new(keys)
            .style(
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(60, 60, 60)),
            )
            .alignment(Alignment::Center);
        f.render_widget(p, area);
    }

    fn render_input_overlay(&self, f: &mut Frame, input: &InputMode) {
        let area = f.area();
        // Center a small popup
        let popup_w = 50u16;
        let popup_h = 5u16;
        let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect::new(popup_x, popup_y, popup_w.min(area.width), popup_h.min(area.height));

        let target_label = match input.target {
            PatchRow::Device => "设备伪装",
            PatchRow::Smbios => "SMBIOS 伪装",
            _ => "",
        };

        let block = Block::default()
            .title(format!(" 自定义机型 — {target_label} "))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Rgb(255, 180, 50)).bg(Color::Rgb(30, 30, 30)));

        let inner = block.inner(popup);
        f.render_widget(block, popup);

        let cursor = if input.buffer.is_empty() {
            "█"
        } else {
            ""
        };
        let display = format!("▶ {}{cursor}", input.buffer);
        let p = Paragraph::new(display)
            .style(Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 30)))
            .alignment(Alignment::Left);
        f.render_widget(p, Rect::new(inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), 1));

        let hint = Paragraph::new("Enter 确认 · Esc 取消")
            .style(Style::default().fg(Color::DarkGray).bg(Color::Rgb(30, 30, 30)))
            .alignment(Alignment::Center);
        f.render_widget(hint, Rect::new(inner.x, inner.y + 2, inner.width, 1));
    }
}
