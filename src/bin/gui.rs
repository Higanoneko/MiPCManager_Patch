//! 小米电脑管家 / 小米互联 增强补丁 — egui 桌面界面。
//!
//! 浅色工具台 + 小米橙点缀；分区：状态 / 补丁 / 安装拖放区 / 日志。
//! 全部操作走 `mipcmanager_patch::ops`，与 CLI 一致。

#![windows_subsystem = "windows"]

#[cfg(not(windows))]
fn main() {
    eprintln!("mipcm_gui 仅支持 Windows。");
}

#[cfg(windows)]
fn load_app_icon() -> std::sync::Arc<egui::IconData> {
    let ico_bytes = include_bytes!("../../assets/MiPCManager.ico");
    let Ok(icon_dir) = ico::IconDir::read(std::io::Cursor::new(ico_bytes.as_ref())) else {
        return std::sync::Arc::new(egui::IconData::default());
    };
    if let Some(entry) = icon_dir.entries().first() {
        if let Ok(image) = entry.decode() {
            return std::sync::Arc::new(egui::IconData {
                rgba: image.rgba_data().to_vec(),
                width: image.width(),
                height: image.height(),
            });
        }
    }
    std::sync::Arc::new(egui::IconData::default())
}

#[cfg(windows)]
fn main() {
    mipcmanager_patch::elevate::ensure_elevated();

    let icon = load_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 1050.0])
            .with_min_inner_size([640.0, 820.0])
            .with_title("MiPCM Patch")
            .with_drag_and_drop(true)
            .with_icon(icon),
        centered: true,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "MiPCM Patch",
        options,
        Box::new(|cc| Ok(Box::new(PatchApp::new(cc)))),
    );
}

#[cfg(windows)]
struct PatchApp {
    status: String,
    log: String,
    log_follow: bool,
    model_idx: usize,
    custom_model: String,
    full_features: bool,
    last_error: Option<String>,
}

#[cfg(windows)]
impl PatchApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let mut app = Self {
            status: String::new(),
            log: String::new(),
            log_follow: true,
            model_idx: 0,
            custom_model: String::new(),
            full_features: false,
            last_error: None,
        };
        app.refresh_status();
        app
    }

    fn push_log(&mut self, line: &str) {
        if !self.log.is_empty() {
            self.log.push('\n');
        }
        self.log.push_str(line);
        self.log_follow = true;
    }

    fn run_op(&mut self, title: &str, result: anyhow::Result<Vec<String>>) {
        self.push_log(&format!("—— {title} ——"));
        match result {
            Ok(lines) => {
                self.last_error = None;
                for line in lines {
                    self.push_log(&line);
                }
            }
            Err(error) => {
                let msg = format!("{error:#}");
                self.last_error = Some(msg.clone());
                self.push_log(&format!("错误：{msg}"));
            }
        }
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        use mipcmanager_patch::ops;
        self.full_features = ops::full_features_available();
        self.status = ops::status_lines().join("\n");
    }

    fn selected_model(&self) -> String {
        use mipcmanager_patch::device_spoof;
        let custom = self.custom_model.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        device_spoof::PRESETS
            .get(self.model_idx)
            .map(|p| p.code.to_string())
            .unwrap_or_else(|| device_spoof::DEFAULT_MODEL.to_string())
    }

    fn install_path(&mut self, path: std::path::PathBuf) {
        use mipcmanager_patch::ops;
        self.push_log(&format!("安装包：{}", path.display()));
        self.run_op("安装", ops::install_from_path(&path));
    }
}

#[cfg(windows)]
impl eframe::App for PatchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use mipcmanager_patch::{device_spoof, ops};

        let ctx = ui.ctx().clone();
        let dropped: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            {
                self.install_path(path);
            } else {
                self.push_log("忽略非 .exe 拖入文件。");
            }
        }

        let accent = egui::Color32::from_rgb(255, 105, 0);
        let ink = egui::Color32::from_rgb(28, 30, 33);
        let muted = egui::Color32::from_rgb(110, 118, 128);
        let panel = egui::Color32::WHITE;
        let soft = egui::Color32::from_rgb(245, 246, 248);
        let line = egui::Color32::from_rgb(226, 229, 233);
        let danger = egui::Color32::from_rgb(200, 55, 48);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(soft).inner_margin(18.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("MiPCM Patch")
                                .size(26.0)
                                .color(ink)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("小米电脑管家 / 小米互联 · 功能增强补丁")
                                .size(13.0)
                                .color(muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ghost_btn(ui, ink, line, "刷新状态").clicked() {
                            self.refresh_status();
                        }
                    });
                });
                ui.add_space(14.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        section_card(ui, panel, line, "安装状态", |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(120.0)
                                .id_salt("status_scroll")
                                .show(ui, |ui| {
                                    ui.monospace(&self.status);
                                });
                            if !self.full_features {
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(
                                        "未检测到完整版 XiaomiPCManager：摄像头 / 音频 / 设备伪装不可用。小米互联可用地区伪装与安装。",
                                    )
                                    .size(12.0)
                                    .color(muted),
                                );
                            }
                        });

                        ui.add_space(12.0);

                        section_card(ui, panel, line, "补丁操作", |ui| {
                            action_row(ui, "地区伪装", |ui| {
                                if primary_btn(ui, accent, "应用").clicked() {
                                    self.run_op(
                                        "地区伪装 · 应用",
                                        ops::apply_locale(None, "CN", true, false),
                                    );
                                }
                                if ghost_btn(ui, ink, line, "还原").clicked() {
                                    self.run_op(
                                        "地区伪装 · 还原",
                                        ops::revert_locale(None, true, false),
                                    );
                                }
                            });
                            ui.add_space(8.0);
                            ui.add_enabled_ui(self.full_features, |ui| {
                                action_row(ui, "抑制摄像头弹窗", |ui| {
                                    if primary_btn(ui, accent, "应用").clicked() {
                                        self.run_op(
                                            "抑制摄像头弹窗 · 应用",
                                            ops::apply_camera(None, false),
                                        );
                                    }
                                    if ghost_btn(ui, ink, line, "还原").clicked() {
                                        self.run_op(
                                            "抑制摄像头弹窗 · 还原",
                                            ops::revert_camera(None, false),
                                        );
                                    }
                                });
                                ui.add_space(8.0);
                                action_row(ui, "音频流转", |ui| {
                                    if primary_btn(ui, accent, "无线 WiFi").clicked() {
                                        self.run_op(
                                            "音频流转 · 无线 WiFi",
                                            ops::apply_audio(
                                                ops::BroadcastMode::Wireless,
                                                None,
                                                false,
                                            ),
                                        );
                                    }
                                    if ghost_btn(ui, ink, line, "有线 LAN").clicked() {
                                        self.run_op(
                                            "音频流转 · 有线 LAN",
                                            ops::apply_audio(
                                                ops::BroadcastMode::Wired,
                                                None,
                                                false,
                                            ),
                                        );
                                    }
                                    if ghost_btn(ui, ink, line, "还原").clicked() {
                                        self.run_op(
                                            "音频流转 · 还原",
                                            ops::revert_audio(None, false),
                                        );
                                    }
                                });
                                ui.add_space(8.0);
                                ui.add_enabled_ui(self.full_features, |ui| {
                                    action_row(ui, "双网卡音频 [实验性]", |ui| {
                                        if ghost_btn(ui, ink, line, "诊断").clicked() {
                                            let dir = match ops::resolve_full_version_dir() {
                                                Ok(d) => d,
                                                Err(e) => {
                                                    self.push_log(&format!("错误：{e:#}"));
                                                    return;
                                                }
                                            };
                                            self.run_op(
                                                "双网卡音频 · 诊断",
                                                mipcmanager_patch::audio_dual_nic::diagnose(&dir),
                                            );
                                        }
                                        if primary_btn(ui, accent, "修复").clicked() {
                                            let dir = match ops::resolve_full_version_dir() {
                                                Ok(d) => d,
                                                Err(e) => {
                                                    self.push_log(&format!("错误：{e:#}"));
                                                    return;
                                                }
                                            };
                                            self.run_op(
                                                "双网卡音频 · 修复",
                                                mipcmanager_patch::audio_dual_nic::auto_fix(&dir),
                                            );
                                        }
                                    });
                                });
                                ui.add_space(8.0);
                                action_row(ui, "设备伪装", |ui| {
                                    egui::ComboBox::from_id_salt("model_combo")
                                        .selected_text(
                                            device_spoof::PRESETS
                                                .get(self.model_idx)
                                                .map(|p| format!("{} · {}", p.code, p.name))
                                                .unwrap_or_default(),
                                        )
                                        .width(220.0)
                                        .show_ui(ui, |ui| {
                                            for (i, p) in device_spoof::PRESETS.iter().enumerate()
                                            {
                                                ui.selectable_value(
                                                    &mut self.model_idx,
                                                    i,
                                                    format!("{} · {}", p.code, p.name),
                                                );
                                            }
                                        });
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.custom_model)
                                            .hint_text("自定义机型")
                                            .desired_width(100.0),
                                    );
                                    if primary_btn(ui, accent, "应用").clicked() {
                                        let model = self.selected_model();
                                        self.run_op(
                                            &format!("设备伪装 · 应用（{model}）"),
                                            ops::apply_device(&model, None, false),
                                        );
                                    }
                                    if ghost_btn(ui, ink, line, "还原").clicked() {
                                        self.run_op(
                                            "设备伪装 · 还原",
                                            ops::revert_device(None, false),
                                        );
                                    }
                                });
                            });
                        });

                        ui.add_space(12.0);

                        section_card(ui, panel, line, "安装小米电脑管家 / 小米互联", |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "启动前写入 SpoofDevice、注入代理，并旁路 Win11 版本门闸与机型白名单。",
                                )
                                .size(12.0)
                                .color(muted),
                            );
                            ui.add_space(10.0);

                            let drop_h = 112.0;
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), drop_h),
                                egui::Sense::click(),
                            );
                            let hovered = response.hovered()
                                || ctx.input(|i| !i.raw.hovered_files.is_empty());
                            let fill = if hovered {
                                egui::Color32::from_rgb(255, 244, 235)
                            } else {
                                soft
                            };
                            let stroke = if hovered {
                                egui::Stroke::new(1.5, accent)
                            } else {
                                egui::Stroke::new(1.0, line)
                            };
                            ui.painter().rect(
                                rect,
                                12.0,
                                fill,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                            let label = if hovered {
                                "松开以开始安装  ·  或点击选择 .exe"
                            } else {
                                "将安装包拖到此处\n或点击选择本地 .exe"
                            };
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                label,
                                egui::FontId::proportional(15.0),
                                if hovered { accent } else { muted },
                            );
                            if response.clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("安装包", &["exe"])
                                    .set_title("选择小米电脑管家 / 小米互联安装包")
                                    .pick_file()
                                {
                                    self.install_path(path);
                                }
                            }
                        });

                        ui.add_space(12.0);

                        section_card(ui, panel, line, "运行日志", |ui| {
                            if let Some(err) = &self.last_error {
                                ui.colored_label(danger, format!("最近错误：{err}"));
                                ui.add_space(6.0);
                            }
                            egui::ScrollArea::vertical()
                                .max_height(190.0)
                                .id_salt("log_scroll")
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.monospace(&self.log);
                                });
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ghost_btn(ui, ink, line, "清空日志").clicked() {
                                    self.log.clear();
                                    self.last_error = None;
                                }
                                ui.label(
                                    egui::RichText::new("新日志自动跟随底部")
                                        .size(11.0)
                                        .color(muted),
                                );
                            });
                        });
                    });
            });
    }
}

#[cfg(windows)]
fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.visuals = egui::Visuals::light();
    style.visuals.window_fill = egui::Color32::from_rgb(245, 246, 248);
    style.visuals.panel_fill = egui::Color32::from_rgb(245, 246, 248);
    style.visuals.widgets.inactive.corner_radius = 8.0.into();
    style.visuals.widgets.hovered.corner_radius = 8.0.into();
    style.visuals.widgets.active.corner_radius = 8.0.into();
    ctx.set_style_of(egui::Theme::Light, style);

    let mut fonts = egui::FontDefinitions::default();
    for path in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cn".into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cn".into());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cn".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(windows)]
fn section_card(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    stroke: egui::Color32,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(14.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.expand_to_include_rect(ui.max_rect());
            ui.label(
                egui::RichText::new(title)
                    .size(14.0)
                    .color(egui::Color32::from_rgb(28, 30, 33))
                    .strong(),
            );
            ui.add_space(10.0);
            add(ui);
        });
}

#[cfg(windows)]
fn action_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(128.0, 32.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .color(egui::Color32::from_rgb(28, 30, 33))
                        .strong(),
                );
            },
        );
        ui.add_space(8.0);
        let remaining = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(remaining, 32.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                add(ui);
            },
        );
    });
}

#[cfg(windows)]
fn primary_btn(ui: &mut egui::Ui, accent: egui::Color32, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(accent)
        .corner_radius(8.0)
        .min_size(egui::vec2(72.0, 32.0)),
    )
}

#[cfg(windows)]
fn ghost_btn(
    ui: &mut egui::Ui,
    ink: egui::Color32,
    line: egui::Color32,
    text: &str,
) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(ink))
            .fill(egui::Color32::WHITE)
            .stroke(egui::Stroke::new(1.0, line))
            .corner_radius(8.0)
            .min_size(egui::vec2(72.0, 32.0)),
    )
}
