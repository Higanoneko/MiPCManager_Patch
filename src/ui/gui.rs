#![windows_subsystem = "windows"]

use anyhow::Result;
use mipcmanager_patch::{elevate, i18n, ops, patches::device as ds};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::PathBuf;
use std::rc::Rc;

slint::include_modules!();

#[cfg(not(windows))]
fn main() {
    eprintln!("MiPCM_GUI 仅支持 Windows。");
}

#[cfg(windows)]
fn main() {
    elevate::ensure_elevated();

    if let Some(message) = ops::close_all_on_startup() {
        let _ = message;
    }

    let lang = i18n::detect_lang();

    let app = AppWindow::new().unwrap();

    app.on_tr(move |key: SharedString| -> SharedString {
        i18n::tr(&key, lang).into()
    });

    let presets: Vec<SharedString> = ds::PRESETS
        .iter()
        .map(|p| SharedString::from(format!("{} · {}", p.code, p.name)))
        .collect();
    let model = Rc::new(VecModel::from(presets));
    app.set_model_presets(ModelRc::from(model));

    refresh(&app);

    setup_callbacks(&app, lang);

    app.run().unwrap();
}

fn refresh(app: &AppWindow) {
    let full = ops::full_features_available();
    app.set_full_features(full);
    let status = ops::status_lines().join("\n");
    app.set_status_text(status.into());
}

fn append_log(app: &AppWindow, label: &str, result: Result<Vec<String>>) {
    let mut log = format!("—— {} ——\n", label);
    match result {
        Ok(lines) => {
            app.set_last_error("".into());
            for line in lines {
                log.push_str(&line);
                log.push('\n');
            }
        }
        Err(e) => {
            let m = format!("{:#}", e);
            log.push_str(&format!("Error: {}\n", m));
            app.set_last_error(m.into());
        }
    }
    let current: String = app.get_log_text().into();
    app.set_log_text(format!("{}{}", current, log).into());
    refresh(app);
}

fn run_patch(app: &AppWindow, label: &str, f: impl FnOnce() -> Result<Vec<String>>) {
    append_log(app, label, f());
}

#[cfg(windows)]
fn setup_callbacks(app: &AppWindow, lang: i18n::Lang) {
    let app_weak = app.as_weak();

    app.on_refresh({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            refresh(&app);
        }
    });

    app.on_apply_locale({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.locale.apply", lang), || ops::apply_locale(None, "CN", true, false));
        }
    });

    app.on_revert_locale({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.locale.revert", lang), || ops::revert_locale(None, true, false));
        }
    });

    app.on_apply_device({
        let app_weak = app_weak.clone();
        move |model: SharedString| {
            let app = app_weak.unwrap();
            let m = model.to_string();
            let label = i18n::tr("gui.op.device.apply", lang).replace("{model}", &m);
            run_patch(&app, &label, || ops::apply_device(&m, None, false));
        }
    });

    app.on_revert_device({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.device.revert", lang), || ops::revert_device(None, false));
        }
    });

    app.on_apply_camera({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.camera.apply", lang), || ops::apply_camera(None, false));
        }
    });

    app.on_revert_camera({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.camera.revert", lang), || ops::revert_camera(None, false));
        }
    });

    app.on_apply_audio_wifi({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.audio.wifi", lang), || ops::apply_audio(ops::BroadcastMode::Wireless, None, false));
        }
    });

    app.on_apply_audio_lan({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.audio.lan", lang), || ops::apply_audio(ops::BroadcastMode::Wired, None, false));
        }
    });

    app.on_revert_audio({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.audio.revert", lang), || ops::revert_audio(None, false));
        }
    });

    app.on_diagnose_dual_nic({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            let label = i18n::tr("gui.op.dualnic.diagnose", lang);
            let dir = match ops::resolve_full_version_dir() {
                Ok(d) => d,
                Err(e) => {
                    append_log(&app, label, Err(e));
                    return;
                }
            };
            run_patch(&app, label, || mipcmanager_patch::experimental::audio_dual_nic::diagnose(&dir));
        }
    });

    app.on_fix_dual_nic({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            let label = i18n::tr("gui.op.dualnic.fix", lang);
            let dir = match ops::resolve_full_version_dir() {
                Ok(d) => d,
                Err(e) => {
                    append_log(&app, label, Err(e));
                    return;
                }
            };
            run_patch(&app, label, || mipcmanager_patch::experimental::audio_dual_nic::auto_fix(&dir));
        }
    });

    app.on_apply_smbios({
        let app_weak = app_weak.clone();
        move |model: SharedString| {
            let app = app_weak.unwrap();
            let m = model.to_string();
            let label = i18n::tr("gui.op.smbios.apply", lang).replace("{model}", &m);
            run_patch(&app, &label, || ops::apply_smbios(Some(&m), None, false));
        }
    });

    app.on_revert_smbios({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.smbios.revert", lang), || ops::revert_smbios(None, false));
        }
    });

    app.on_clear_log({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            app.set_log_text("".into());
            app.set_last_error("".into());
        }
    });

    app.on_uninstall_msix({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            run_patch(&app, i18n::tr("gui.op.uninstall.msix", lang), || ops::uninstall_msix(false));
        }
    });

    app.on_request_uninstall({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            match ops::uninstall_product_description() {
                Ok(d) => {
                    app.set_confirm_desc(d.into());
                    app.set_show_confirm(true);
                }
                Err(e) => {
                    append_log(&app, i18n::tr("gui.op.uninstall.product", lang), Err(e));
                }
            }
        }
    });

    app.on_confirm_uninstall({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            app.set_show_confirm(false);
            app.set_confirm_desc("".into());
            run_patch(&app, i18n::tr("gui.op.uninstall.product", lang), ops::uninstall_product);
        }
    });

    app.on_cancel_uninstall({
        let app_weak = app_weak.clone();
        move || {
            let app = app_weak.unwrap();
            app.set_show_confirm(false);
            app.set_confirm_desc("".into());
        }
    });

    app.on_download_and_install({
        let app_weak = app_weak.clone();
        move |url: SharedString| {
            let url = url.to_string();
            if url.trim().is_empty() {
                return;
            }
            let app = app_weak.unwrap();
            app.set_downloading(true);
            let log = format!("{}\n", i18n::tr("gui.downloading.start", lang).replace("{url}", &url));
            let current: String = app.get_log_text().into();
            app.set_log_text(format!("{}{}", current, log).into());

            let app_weak2 = app_weak.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<PathBuf> {
                    use mipcmanager_patch::install::pc_manager_installer;
                    let dir = pc_manager_installer::patcher_dir()?;
                    pc_manager_installer::download_installer(&url, &dir)
                })();

                let _ = slint::invoke_from_event_loop(move || {
                    let app = app_weak2.unwrap();
                    app.set_downloading(false);
                    match result {
                        Ok(path) => {
                            let path_str = path.display().to_string();
                            let label = i18n::tr("gui.op.install", lang).replace("{path}", &path_str);
                            append_log(&app, &label, ops::install_from_path(&path));
                        }
                        Err(e) => {
                            append_log(&app, i18n::tr("install.download.and.install", lang), Err(e));
                        }
                    }
                });
            });
        }
    });

    app.on_browse_installer({
        let app_weak = app_weak.clone();
        move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(i18n::tr("gui.browse.filter", lang), &["exe"])
                .set_title(i18n::tr("gui.browse.title", lang))
                .pick_file()
            {
                let app = app_weak.unwrap();
                app.set_path_input(path.display().to_string().into());
            }
        }
    });

    app.on_start_install({
        let app_weak = app_weak.clone();
        move |path: SharedString| {
            let app = app_weak.unwrap();
            let p = PathBuf::from(path.to_string());
            if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                return;
            }
            let path_str = p.display().to_string();
            let label = i18n::tr("gui.op.install", lang).replace("{path}", &path_str);
            append_log(&app, &label, ops::install_from_path(&p));
            app.set_path_input("".into());
        }
    });
}
