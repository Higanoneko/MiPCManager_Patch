//! TUI 应用状态与事件循环。

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph},
};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::{
    experimental::audio_dual_nic,
    i18n::{self, Lang},
    ops::{self, BroadcastMode},
    patches::device::PRESETS,
};

use super::{theme, widgets};

// ── Log message ───────────────────────────────────────────────────
pub enum LogMessage {
    Line(String),
    Done,
    Error(String),
}

// ── Tabs ───────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Patches,
    Install,
    Uninstall,
    Log,
}

impl Tab {
    fn all() -> &'static [Tab] {
        &[Tab::Patches, Tab::Install, Tab::Uninstall, Tab::Log]
    }

    fn label(self, lang: Lang) -> &'static str {
        match self {
            Tab::Patches => i18n::tr("tui.tab.patches", lang),
            Tab::Install => i18n::tr("tui.tab.install", lang),
            Tab::Uninstall => i18n::tr("tui.tab.uninstall", lang),
            Tab::Log => i18n::tr("tui.tab.log", lang),
        }
    }

    fn next(self) -> Tab {
        let all = Self::all();
        let idx = all.iter().position(|t| *t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    fn prev(self) -> Tab {
        let all = Self::all();
        let idx = all.iter().position(|t| *t == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }
}

// ── Patch row ────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchRow {
    Locale,
    Camera,
    Hotkey,
    Audio,
    Device,
    Xiaoai,
    Smbios,
    DualNic,
}

impl PatchRow {
    fn all() -> &'static [PatchRow] {
        &[
            PatchRow::Locale,
            PatchRow::Camera,
            PatchRow::Hotkey,
            PatchRow::Audio,
            PatchRow::Device,
            PatchRow::Xiaoai,
            PatchRow::Smbios,
            PatchRow::DualNic,
        ]
    }

    fn label(&self, lang: Lang) -> &'static str {
        match self {
            PatchRow::Locale => i18n::tr("patch.locale.detail", lang),
            PatchRow::Camera => i18n::tr("patch.camera.detail", lang),
            PatchRow::Hotkey => i18n::tr("patch.hotkey.detail", lang),
            PatchRow::Audio => i18n::tr("patch.audio.detail", lang),
            PatchRow::Device => i18n::tr("patch.device.detail", lang),
            PatchRow::Xiaoai => i18n::tr("patch.xiaoai.detail", lang),
            PatchRow::Smbios => i18n::tr("patch.smbios.detail", lang),
            PatchRow::DualNic => i18n::tr("patch.dualnic.detail", lang),
        }
    }

    fn desc(&self, lang: Lang) -> &'static str {
        match self {
            PatchRow::Locale => i18n::tr("patch.locale.desc", lang),
            PatchRow::Camera => i18n::tr("patch.camera.desc", lang),
            PatchRow::Hotkey => i18n::tr("patch.hotkey.desc", lang),
            PatchRow::Audio => i18n::tr("patch.audio.desc", lang),
            PatchRow::Device => i18n::tr("patch.device.desc", lang),
            PatchRow::Xiaoai => i18n::tr("patch.xiaoai.desc", lang),
            PatchRow::Smbios => i18n::tr("patch.smbios.desc", lang),
            PatchRow::DualNic => i18n::tr("patch.dualnic.desc", lang),
        }
    }

    fn btn_labels(&self, lang: Lang) -> Vec<String> {
        match self {
            PatchRow::Audio => vec![
                i18n::tr("btn.wifi", lang).to_string(),
                i18n::tr("btn.lan", lang).to_string(),
                i18n::tr("btn.revert", lang).to_string(),
            ],
            PatchRow::DualNic => vec![
                i18n::tr("btn.dualnic.diagnose", lang).to_string(),
                i18n::tr("btn.dualnic.fix", lang).to_string(),
            ],
            _ => vec![
                i18n::tr("btn.apply", lang).to_string(),
                i18n::tr("btn.revert", lang).to_string(),
            ],
        }
    }

    fn btn_execute(&self, idx: usize, app: &App, lang: Lang, tx: &Sender<LogMessage>) {
        match (self, idx) {
            (PatchRow::Locale, 0) => {
                let label = i18n::tr("tui.op.locale.apply", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::apply_locale(None, "CN", true, false)
                });
            }
            (PatchRow::Locale, 1) => {
                let label = i18n::tr("tui.op.locale.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::revert_locale(None, true, false)
                });
            }
            (PatchRow::Camera, 0) => {
                let label = i18n::tr("tui.op.camera.apply", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::apply_camera(None, false));
            }
            (PatchRow::Camera, 1) => {
                let label = i18n::tr("tui.op.camera.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::revert_camera(None, false));
            }
            (PatchRow::Hotkey, 0) => {
                let label = i18n::tr("tui.op.hotkey.apply", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::apply_mouse_hotkeys(None, false)
                });
            }
            (PatchRow::Hotkey, 1) => {
                let label = i18n::tr("tui.op.hotkey.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::revert_mouse_hotkeys(None, false)
                });
            }
            (PatchRow::Audio, 0) => {
                let label = i18n::tr("tui.op.audio.wifi", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::apply_audio(BroadcastMode::Wireless, None, false)
                });
            }
            (PatchRow::Audio, 1) => {
                let label = i18n::tr("tui.op.audio.lan", lang).to_string();
                spawn_op(tx.clone(), label, lang, || {
                    ops::apply_audio(BroadcastMode::Wired, None, false)
                });
            }
            (PatchRow::Audio, 2) => {
                let label = i18n::tr("tui.op.audio.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::revert_audio(None, false));
            }
            (PatchRow::Device, 0) => {
                let model = app.current_device_model().to_string();
                let label = i18n::tr("tui.op.device.apply", lang).replace("{model}", &model);
                spawn_op(tx.clone(), label, lang, move || {
                    ops::apply_device(&model, None, false)
                });
            }
            (PatchRow::Device, 1) => {
                let label = i18n::tr("tui.op.device.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::revert_device(None, false));
            }
            (PatchRow::Xiaoai, 0) => {
                let label = i18n::tr("tui.op.xiaoai.apply", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::apply_xiaoai(None, false));
            }
            (PatchRow::Xiaoai, 1) => {
                let label = i18n::tr("tui.op.xiaoai.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::revert_xiaoai(None, false));
            }
            (PatchRow::Smbios, 0) => {
                let model = app.current_smbios_model().to_string();
                let label = i18n::tr("tui.op.smbios.apply", lang).replace("{model}", &model);
                spawn_op(tx.clone(), label, lang, move || {
                    ops::apply_smbios(Some(&model), None, false)
                });
            }
            (PatchRow::Smbios, 1) => {
                let label = i18n::tr("tui.op.smbios.revert", lang).to_string();
                spawn_op(tx.clone(), label, lang, || ops::revert_smbios(None, false));
            }
            (PatchRow::DualNic, 0) => {
                let label = "双网卡诊断".to_string();
                spawn_op(tx.clone(), label, lang, || {
                    audio_dual_nic::diagnose(&ops::resolve_full_version_dir()?)
                });
            }
            (PatchRow::DualNic, 1) => {
                let label = "双网卡修复".to_string();
                spawn_op(tx.clone(), label, lang, || {
                    audio_dual_nic::auto_fix(&ops::resolve_full_version_dir()?)
                });
            }
            _ => {}
        }
    }
}

// ── Uninstall row ────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
enum UninstallRow {
    Msix,
    Product,
}

impl UninstallRow {
    fn all() -> &'static [UninstallRow] {
        &[UninstallRow::Msix, UninstallRow::Product]
    }

    fn label(&self, lang: Lang) -> &'static str {
        match self {
            UninstallRow::Msix => i18n::tr("tui.op.uninstall.msix", lang),
            UninstallRow::Product => i18n::tr("tui.op.uninstall.product", lang),
        }
    }
}

// ── Overlay modes ────────────────────────────────────────────────
struct InputMode {
    target: PatchRow,
    buffer: String,
}

struct ConfirmMode {
    message: String,
    action_label: String,
}

// ── App state ────────────────────────────────────────────────────
pub struct App {
    lang: Lang,
    tx: Sender<LogMessage>,

    tab: Tab,
    patch_idx: usize,
    patch_btn_idx: usize,
    uninstall_idx: usize,
    log_scroll: usize,

    device_preset_idx: usize,
    smbios_preset_idx: usize,
    custom_device_model: Option<String>,
    custom_smbios_model: Option<String>,

    input_mode: Option<InputMode>,
    confirm_mode: Option<ConfirmMode>,

    status_lines: Vec<String>,
    full_features: bool,

    log: Vec<String>,
    op_running: bool,
    op_label: String,
}

impl App {
    pub fn new(tx: Sender<LogMessage>, lang: Lang) -> Self {
        let mut app = Self {
            lang,
            tx,
            tab: Tab::Patches,
            patch_idx: 0,
            patch_btn_idx: 0,
            uninstall_idx: 0,
            log_scroll: 0,
            device_preset_idx: 0,
            smbios_preset_idx: 0,
            custom_device_model: None,
            custom_smbios_model: None,
            input_mode: None,
            confirm_mode: None,
            status_lines: Vec::new(),
            full_features: false,
            log: Vec::new(),
            op_running: false,
            op_label: String::new(),
        };
        app.refresh_status();
        app
    }

    // ── Public: Event loop ─────────────────────────────────────
    pub fn run_loop(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
        rx: Receiver<LogMessage>,
    ) -> Result<()> {
        let tick = Duration::from_millis(50);
        loop {
            self.drain_log(&rx);
            let _ = terminal.draw(|f| self.render(f));

            if event::poll(tick)? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        if !self.handle_key(key.code) {
                            return Ok(());
                        }
                    }
                    Event::Mouse(mouse)
                        if mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
                    {
                        let _ = self.handle_mouse(mouse.column, mouse.row);
                    }
                    _ => {}
                }
            }
        }
    }

    // ── Keyboard ───────────────────────────────────────────────
    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.input_mode.is_some() {
            return self.handle_input_key(code);
        }
        if self.confirm_mode.is_some() {
            return self.handle_confirm_key(code);
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return false,
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.patch_btn_idx = 0;
                return true;
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                self.patch_btn_idx = 0;
                return true;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_status();
                return true;
            }
            _ => {}
        }

        if self.op_running {
            return true;
        }

        match self.tab {
            Tab::Patches => self.handle_patches_key(code),
            Tab::Uninstall => self.handle_uninstall_key(code),
            Tab::Log => self.handle_log_key(code),
            Tab::Install => self.handle_install_key(code),
        }
        true
    }

    fn handle_mouse(&mut self, _col: u16, _row: u16) -> bool {
        true
    }

    fn handle_input_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Esc => self.input_mode = None,
            KeyCode::Enter => {
                if let Some(input) = self.input_mode.take() {
                    let model = input.buffer.trim().to_string();
                    if !model.is_empty() {
                        match input.target {
                            PatchRow::Device => {
                                self.custom_device_model = Some(model.clone());
                                self.log.push(
                                    i18n::tr("tui.log.model.set", self.lang)
                                        .replace("{model}", &model),
                                );
                                self.log_scroll = self.log.len().saturating_sub(1);
                            }
                            PatchRow::Smbios => {
                                self.custom_smbios_model = Some(model.clone());
                                self.log.push(
                                    i18n::tr("tui.log.smbios.set", self.lang)
                                        .replace("{model}", &model),
                                );
                                self.log_scroll = self.log.len().saturating_sub(1);
                            }
                            _ => {}
                        }
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
        true
    }

    fn handle_confirm_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(confirm) = self.confirm_mode.take()
                    && confirm.action_label == "uninstall_product"
                {
                    let label = i18n::tr("tui.op.uninstall.product", self.lang).to_string();
                    spawn_op(self.tx.clone(), label, self.lang, ops::uninstall_product);
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm_mode = None;
            }
            _ => {}
        }
        true
    }

    fn handle_patches_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => {
                if self.patch_idx > 0 {
                    self.patch_idx -= 1;
                    self.patch_btn_idx = 0;
                }
            }
            KeyCode::Down => {
                if self.patch_idx < PatchRow::all().len() - 1 {
                    self.patch_idx += 1;
                    self.patch_btn_idx = 0;
                }
            }
            KeyCode::Left => {
                if self.patch_btn_idx > 0 {
                    self.patch_btn_idx -= 1;
                }
            }
            KeyCode::Right => {
                let patch = PatchRow::all()[self.patch_idx];
                let max = patch.btn_labels(self.lang).len().saturating_sub(1);
                if self.patch_btn_idx < max {
                    self.patch_btn_idx += 1;
                }
            }
            KeyCode::Enter => {
                let patch = PatchRow::all()[self.patch_idx];
                let tx = self.tx.clone();
                patch.btn_execute(self.patch_btn_idx, self, self.lang, &tx);
            }
            KeyCode::Char('m') | KeyCode::Char('M') => self.cycle_preset(),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                let patch = PatchRow::all()[self.patch_idx];
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

    fn handle_uninstall_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => {
                if self.uninstall_idx > 0 {
                    self.uninstall_idx -= 1;
                }
            }
            KeyCode::Down => {
                if self.uninstall_idx < UninstallRow::all().len() - 1 {
                    self.uninstall_idx += 1;
                }
            }
            KeyCode::Enter => match UninstallRow::all()[self.uninstall_idx] {
                UninstallRow::Msix => {
                    let label = i18n::tr("tui.op.uninstall.msix", self.lang).to_string();
                    spawn_op(self.tx.clone(), label, self.lang, || {
                        ops::uninstall_msix(false)
                    });
                }
                UninstallRow::Product => match ops::uninstall_product_description() {
                    Ok(desc) => {
                        self.confirm_mode = Some(ConfirmMode {
                            message: desc,
                            action_label: "uninstall_product".to_string(),
                        });
                    }
                    Err(e) => {
                        self.log.push(
                            i18n::tr("tui.log.uninstall.fetch.error", self.lang)
                                .replace("{error}", &format!("{e:#}")),
                        );
                        self.log_scroll = self.log.len().saturating_sub(1);
                    }
                },
            },
            _ => {}
        }
    }

    fn handle_install_key(&mut self, code: KeyCode) {
        if code == KeyCode::Enter {
            let label = i18n::tr("tui.op.xiaoai.install", self.lang).to_string();
            spawn_op(self.tx.clone(), label, self.lang, ops::install_local_xiaoai);
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
            KeyCode::Home => self.log_scroll = 0,
            KeyCode::End => self.log_scroll = self.log.len().saturating_sub(1),
            _ => {}
        }
    }

    // ── Helpers ────────────────────────────────────────────────
    fn current_device_model(&self) -> &str {
        self.custom_device_model
            .as_deref()
            .unwrap_or(PRESETS[self.device_preset_idx].code)
    }

    fn current_smbios_model(&self) -> &str {
        self.custom_smbios_model
            .as_deref()
            .unwrap_or(PRESETS[self.smbios_preset_idx].code)
    }

    fn cycle_preset(&mut self) {
        let patch = PatchRow::all()[self.patch_idx];
        let lang = self.lang;
        match patch {
            PatchRow::Device => {
                self.custom_device_model = None;
                self.device_preset_idx = (self.device_preset_idx + 1) % PRESETS.len();
                self.log.push(
                    i18n::tr("tui.log.model.switch", lang)
                        .replace("{code}", PRESETS[self.device_preset_idx].code)
                        .replace("{name}", PRESETS[self.device_preset_idx].name),
                );
                self.log_scroll = self.log.len().saturating_sub(1);
            }
            PatchRow::Smbios => {
                self.custom_smbios_model = None;
                self.smbios_preset_idx = (self.smbios_preset_idx + 1) % PRESETS.len();
                self.log.push(
                    i18n::tr("tui.log.smbios.switch", lang)
                        .replace("{code}", PRESETS[self.smbios_preset_idx].code)
                        .replace("{name}", PRESETS[self.smbios_preset_idx].name),
                );
                self.log_scroll = self.log.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    fn refresh_status(&mut self) {
        self.status_lines = ops::status_lines();
        self.full_features = ops::full_features_available();
    }

    fn drain_log(&mut self, rx: &Receiver<LogMessage>) {
        loop {
            match rx.try_recv() {
                Ok(LogMessage::Line(line)) => {
                    self.log.push(line);
                    self.log_scroll = self.log.len().saturating_sub(1);
                }
                Ok(LogMessage::Done) => {
                    self.op_running = false;
                    self.op_label.clear();
                    self.refresh_status();
                }
                Ok(LogMessage::Error(e)) => {
                    self.log.push(format!("✗ {e}"));
                    self.op_running = false;
                    self.op_label.clear();
                    self.log_scroll = self.log.len().saturating_sub(1);
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    // ── Rendering ──────────────────────────────────────────────
    fn render(&self, f: &mut Frame) {
        let area = f.area();
        let bg = Block::default().style(Style::default().bg(theme::BG));
        f.render_widget(bg, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(5),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_header(f, chunks[0]);
        let tab_labels: Vec<&str> = Tab::all().iter().map(|t| t.label(self.lang)).collect();
        widgets::draw_tabs(f, chunks[1], &tab_labels, self.tab as usize);

        match self.tab {
            Tab::Patches => self.render_patches(f, chunks[2]),
            Tab::Install => self.render_install(f, chunks[2]),
            Tab::Uninstall => self.render_uninstall(f, chunks[2]),
            Tab::Log => self.render_log_full(f, chunks[2]),
        }

        widgets::draw_mini_log(f, chunks[3], &self.log);
        self.render_shortcuts(f, chunks[4]);

        if let Some(ref confirm) = self.confirm_mode {
            widgets::draw_confirm_overlay(
                f,
                area,
                i18n::tr("tui.overlay.confirm", self.lang),
                &confirm.message,
                i18n::tr("tui.overlay.confirm.yn", self.lang),
            );
        }
        if let Some(ref input) = self.input_mode {
            let target_label = match input.target {
                PatchRow::Device => i18n::tr("tui.input.device", self.lang),
                PatchRow::Smbios => i18n::tr("tui.input.smbios", self.lang),
                _ => "",
            };
            let title =
                i18n::tr("tui.overlay.input.title", self.lang).replace("{target}", target_label);
            widgets::draw_input_overlay(
                f,
                area,
                &title,
                &input.buffer,
                i18n::tr("tui.overlay.input.hint", self.lang),
            );
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = format!(" MiPCM Patch v{}", env!("CARGO_PKG_VERSION"));
        let mut title_spans = vec![Span::styled(title, theme::header_title())];

        if self.op_running {
            title_spans.push(Span::styled(
                format!("  ⏳ {}…", self.op_label),
                Style::default().fg(theme::YELLOW).bg(theme::ACCENT_DIM),
            ));
        }

        f.render_widget(
            Paragraph::new(Line::from(title_spans)),
            Rect::new(area.x, area.y, area.width, 1),
        );

        let status_icon = if self.full_features {
            Span::styled(
                format!(" ✓ {} ", i18n::tr("tui.header.ready", self.lang)),
                theme::header_status_icon(true),
            )
        } else {
            Span::styled(
                format!(" ⚠ {} ", i18n::tr("tui.header.limited", self.lang)),
                theme::header_status_icon(false),
            )
        };

        let ver = self.status_lines.first().cloned().unwrap_or_default();
        let ver_short = if ver.len() > 30 {
            format!("{}…", &ver[..27])
        } else {
            ver
        };

        let right_line = Line::from(vec![
            Span::styled(
                format!(" {ver_short} "),
                Style::default().fg(theme::MUTED).bg(theme::ACCENT_DIM),
            ),
            status_icon,
        ]);
        let right_w = right_line.width() as u16;
        f.render_widget(
            Paragraph::new(right_line).alignment(Alignment::Right),
            Rect::new(
                area.x + area.width.saturating_sub(right_w),
                area.y,
                right_w,
                1,
            ),
        );

        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(theme::ACCENT).bg(theme::ACCENT_DIM),
            )])),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }

    fn render_patches(&self, f: &mut Frame, area: Rect) {
        let block = widgets::draw_content_block("补丁操作", true);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let row_h = 4u16;
        let visible = inner.height as usize / row_h as usize;

        let scroll = if PatchRow::all().len() > visible {
            self.patch_idx.saturating_sub(visible.saturating_sub(1))
        } else {
            0
        };

        for (i, patch) in PatchRow::all().iter().enumerate().skip(scroll) {
            let row_y = inner.y + ((i - scroll) as u16 * row_h);
            if row_y + row_h > inner.y + inner.height {
                break;
            }
            let row_area = Rect::new(inner.x, row_y, inner.width, row_h);
            self.render_one_patch_row(f, row_area, patch, i);
        }
    }

    fn render_one_patch_row(&self, f: &mut Frame, area: Rect, patch: &PatchRow, idx: usize) {
        let is_sel = idx == self.patch_idx;
        let lang = self.lang;
        let labels = patch.btn_labels(lang);
        let btns: Vec<(bool, &str)> = labels.iter().map(|l| (true, l.as_str())).collect();

        let extra = if is_sel {
            match patch {
                PatchRow::Device => {
                    let model = self.current_device_model();
                    let preset = &PRESETS[self.device_preset_idx];
                    let c = if self.custom_device_model.is_some() {
                        " (自定义)"
                    } else {
                        ""
                    };
                    vec![Line::from(vec![
                        Span::styled(
                            format!("  机型: {model} · {}{c}", preset.name),
                            Style::default().fg(theme::CYAN),
                        ),
                        Span::styled("  [M]切换  [C]自定义", theme::item_hint()),
                    ])]
                }
                PatchRow::Smbios => {
                    let model = self.current_smbios_model();
                    let preset = &PRESETS[self.smbios_preset_idx];
                    let c = if self.custom_smbios_model.is_some() {
                        " (自定义)"
                    } else {
                        ""
                    };
                    vec![Line::from(vec![
                        Span::styled(
                            format!("  机型: {model} · {}{c}", preset.name),
                            Style::default().fg(theme::PURPLE),
                        ),
                        Span::styled("  [M]切换  [C]自定义", theme::item_hint()),
                    ])]
                }
                _ => vec![],
            }
        } else {
            vec![]
        };

        widgets::draw_patch_row(
            area,
            f,
            patch.label(lang),
            patch.desc(lang),
            is_sel,
            &btns,
            if is_sel { self.patch_btn_idx } else { 0 },
            if extra.is_empty() { None } else { Some(&extra) },
        );
    }

    fn render_install(&self, f: &mut Frame, area: Rect) {
        let block = widgets::draw_content_block("安装", true);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let lang = self.lang;
        let lines = vec![
            Line::from(Span::styled(
                i18n::tr("tui.install.xiaoai.enter", lang),
                theme::item_selected(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                i18n::tr("tui.install.title", lang),
                theme::item_hint(),
            )),
            Line::from(Span::styled(
                i18n::tr("tui.install.hint.cmd1", lang),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                i18n::tr("tui.install.hint.cmd2", lang),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(""),
            Line::from(Span::styled(
                i18n::tr("install.xiaoai.title", lang),
                theme::item_hint(),
            )),
            Line::from(Span::styled(
                i18n::tr("tui.install.hint.xiaoai1", lang),
                Style::default()
                    .fg(theme::PURPLE)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                i18n::tr("tui.install.hint.xiaoai2", lang),
                Style::default()
                    .fg(theme::PURPLE)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                i18n::tr("install.xiaoai.note", lang),
                theme::item_hint(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                i18n::tr("tui.install.desc", lang),
                theme::item_hint(),
            )),
        ];

        f.render_widget(
            Paragraph::new(Text::from(lines))
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme::BG)),
            inner,
        );
    }

    fn render_uninstall(&self, f: &mut Frame, area: Rect) {
        let block = widgets::draw_content_block("卸载", true);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let lang = self.lang;
        let items: Vec<ListItem> = UninstallRow::all()
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let is_sel = i == self.uninstall_idx;
                let style = if is_sel {
                    theme::item_selected()
                } else {
                    theme::item_normal()
                };
                let marker = if is_sel { "▸" } else { " " };
                let desc = match row {
                    UninstallRow::Msix => i18n::tr("tui.uninstall.msix.desc", lang),
                    UninstallRow::Product => i18n::tr("tui.uninstall.product.desc", lang),
                };
                ListItem::new(vec![
                    Line::from(Span::styled(format!("{marker} {}", row.label(lang)), style)),
                    Line::from(Span::styled(format!("   {desc}"), theme::item_hint())),
                    Line::from(""),
                ])
            })
            .collect();

        f.render_widget(List::new(items), inner);
    }

    fn render_log_full(&self, f: &mut Frame, area: Rect) {
        widgets::draw_log_panel(f, area, &self.log, self.log_scroll, true);
    }

    fn render_shortcuts(&self, f: &mut Frame, area: Rect) {
        let lang = self.lang;
        let text = if self.input_mode.is_some() {
            i18n::tr("tui.shortcuts.input", lang)
        } else if self.confirm_mode.is_some() {
            i18n::tr("tui.shortcuts.confirm", lang)
        } else {
            match self.tab {
                Tab::Patches => i18n::tr("tui.shortcuts.patches", lang),
                Tab::Install => i18n::tr("tui.shortcuts.install", lang),
                Tab::Uninstall => i18n::tr("tui.shortcuts.uninstall", lang),
                Tab::Log => i18n::tr("tui.shortcuts.log", lang),
            }
        };

        f.render_widget(
            Paragraph::new(text)
                .style(theme::shortcut_bar())
                .alignment(Alignment::Center),
            area,
        );
    }
}

// ── Free-standing spawn ─────────────────────────────────────────
fn spawn_op<F>(tx: Sender<LogMessage>, label: String, _lang: Lang, f: F)
where
    F: FnOnce() -> Result<Vec<String>> + Send + 'static,
{
    std::thread::spawn(move || {
        let _ = tx.send(LogMessage::Line(format!("—— {label} ——")));
        match f() {
            Ok(lines) => {
                for line in lines {
                    let _ = tx.send(LogMessage::Line(line));
                }
                let _ = tx.send(LogMessage::Line(format!("✓ {label} 完成")));
            }
            Err(e) => {
                let _ = tx.send(LogMessage::Error(format!("✗ {label}: {e:#}")));
            }
        }
        let _ = tx.send(LogMessage::Done);
    });
}
