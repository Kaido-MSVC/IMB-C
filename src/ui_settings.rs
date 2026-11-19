//! 將設定視窗連接至偏好模型，並管理受控的 iat 版本探測。
use crate::{AppWindow, Palette, SettingsWindow, WorkGuard, engine, i18n, iat, preferences};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

#[derive(Default)]
pub(crate) struct Probe {
    cancel: Arc<AtomicBool>,
    worker: RefCell<Option<JoinHandle<()>>>,
}

impl Probe {
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn shutdown(&self) {
        self.cancel();
        if let Some(worker) = self.worker.borrow_mut().take() {
            let _ = worker.join();
        }
    }
}

pub(crate) fn connect(
    app: &AppWindow,
    settings: &SettingsWindow,
    prefs_path: &Path,
    prefs: &Rc<RefCell<preferences::Preferences>>,
    update_auxiliary: impl Fn(&AppWindow) + 'static,
) -> Rc<Probe> {
    let mut last_saved = prefs.borrow().clone();
    app.set_language_index(last_saved.language);
    app.set_theme_index(last_saved.theme);
    app.set_format_index(last_saved.format);
    app.set_avif_quality(last_saved.avif_quality);
    app.set_avif_lossless(last_saved.avif_lossless);
    app.set_jxl_quality(last_saved.jxl_quality);
    app.set_jxl_lossless(last_saved.jxl_lossless);
    app.set_webp_lossless(last_saved.webp_lossless);
    if last_saved.output_folder.is_dir() {
        app.set_output_path(engine::display_path(&last_saved.output_folder).into());
    }
    update_paths(settings, &last_saved);
    settings.set_language_index(last_saved.language);
    settings.set_theme_index(last_saved.theme);
    settings.set_notify(last_saved.notify);
    settings.set_iat_enabled(last_saved.iat_enabled);
    localize(app, settings);
    update_auxiliary(app);

    let aw = app.as_weak();
    let sw = settings.as_weak();
    let path = prefs_path.to_owned();
    let stored = prefs.clone();
    app.on_preferences_changed(move || {
        let a = aw.unwrap();
        let w = sw.unwrap();
        let mut p = stored.borrow_mut();
        p.format = a.get_format_index().clamp(0, 7);
        p.avif_quality = a.get_avif_quality().clamp(1, 100);
        p.avif_lossless = a.get_avif_lossless();
        p.jxl_quality = a.get_jxl_quality().clamp(1, 99);
        p.jxl_lossless = a.get_jxl_lossless();
        p.webp_lossless = a.get_webp_lossless();
        update_paths(&w, &p);
        let error = match preferences::save_changes(&path, &last_saved, &p) {
            Ok(()) => {
                last_saved = p.clone();
                String::new()
            }
            Err(error) => error.to_string(),
        };
        a.set_save_failed(!error.is_empty());
        w.set_save_failed(!error.is_empty());
        w.set_save_error(error.into());
    });

    let aw = app.as_weak();
    let sw = settings.as_weak();
    let stored = prefs.clone();
    settings.on_preferences_changed(move || {
        let a = aw.unwrap();
        let w = sw.unwrap();
        let language = w.get_language_index().clamp(0, 3);
        let theme = w.get_theme_index().clamp(0, 2);
        let appearance_changed = language != a.get_language_index() || theme != a.get_theme_index();
        {
            let mut p = stored.borrow_mut();
            p.language = language;
            p.theme = theme;
            p.notify = w.get_notify();
            p.iat_enabled = w.get_iat_enabled();
        }
        if appearance_changed {
            a.set_language_index(language);
            a.set_theme_index(theme);
            localize(&a, &w);
            update_auxiliary(&a);
        }
        a.invoke_preferences_changed();
    });

    let aw = app.as_weak();
    let stored = prefs.clone();
    settings.on_clear_folders(move || {
        let a = aw.unwrap();
        if a.get_busy() {
            return;
        }
        {
            let mut p = stored.borrow_mut();
            p.source_folder.clear();
            p.output_folder.clear();
        }
        a.set_output_path("".into());
        a.set_ready(false);
        a.invoke_preferences_changed();
        a.set_log(crate::append_log(&a.get_log(), i18n::gui(a.get_language_index(), 50)).into());
    });

    let sw = settings.as_weak();
    app.on_settings(move || {
        let w = sw.unwrap();
        if w.show().is_ok() {
            w.window().with_winit_window(|parent| parent.focus_window());
        }
    });
    let sw = settings.as_weak();
    settings.on_dismiss(move || {
        let _ = sw.unwrap().hide();
    });

    let sw = settings.as_weak();
    let aw = app.as_weak();
    let stored = prefs.clone();
    settings.on_choose_iat(move || {
        let w = sw.unwrap();
        if let Some(path) = w
            .window()
            .with_winit_window(|parent| {
                rfd::FileDialog::new()
                    .set_parent(parent)
                    .add_filter("iat.exe", &["exe"])
                    .pick_file()
            })
            .flatten()
        {
            if !path.is_absolute() || !path.is_file() {
                set_probe_status(&w, Err(i18n::gui(w.get_language_index(), 40).into()));
                return;
            }
            stored.borrow_mut().iat_path = path;
            reset_probe_status(&w);
            aw.unwrap().invoke_preferences_changed();
        }
    });
    let sw = settings.as_weak();
    let aw = app.as_weak();
    let stored = prefs.clone();
    settings.on_reset_iat(move || {
        stored.borrow_mut().iat_path.clear();
        reset_probe_status(&sw.unwrap());
        aw.unwrap().invoke_preferences_changed();
    });

    let probe = Rc::new(Probe::default());
    let work = probe.clone();
    let sw = settings.as_weak();
    let stored = prefs.clone();
    settings.on_probe_iat(move || {
        let w = sw.unwrap();
        if w.get_iat_probing() || w.get_busy() {
            return;
        }
        if let Some(previous) = work.worker.borrow_mut().take() {
            let _ = previous.join();
        }
        reset_probe_status(&w);
        w.set_iat_probing(true);
        let path = stored.borrow().iat_path.clone();
        work.cancel.store(false, Ordering::Relaxed);
        let cancel = work.cancel.clone();
        let sw = sw.clone();
        let guard = WorkGuard::new();
        *work.worker.borrow_mut() = Some(std::thread::spawn(move || {
            let _guard = guard;
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| iat::Client::probe(Some(&path), &exe, &cancel))
                .map(|(_, version)| version);
            let _ = sw.upgrade_in_event_loop(move |w| {
                w.set_iat_probing(false);
                if !cancel.load(Ordering::Relaxed) {
                    set_probe_status(&w, result);
                }
            });
        }));
    });
    probe
}

pub(crate) fn set_probe_status(w: &SettingsWindow, result: Result<(u64, u64, u64), String>) {
    w.set_iat_checked(true);
    match result {
        Ok((major, minor, patch)) => {
            w.set_iat_version(format!("{major}.{minor}.{patch}").into());
            w.set_iat_error("".into());
        }
        Err(error) => {
            w.set_iat_version("".into());
            w.set_iat_error(error.into());
        }
    }
}

fn reset_probe_status(w: &SettingsWindow) {
    w.set_iat_checked(false);
    w.set_iat_version("".into());
    w.set_iat_error("".into());
}

fn update_paths(w: &SettingsWindow, p: &preferences::Preferences) {
    let show = |path: &PathBuf| {
        if path.as_os_str().is_empty() {
            SharedString::default()
        } else {
            engine::display_path(path).into()
        }
    };
    w.set_iat_path(show(&p.iat_path));
    w.set_output_path(show(&p.output_folder));
}

pub(crate) fn palette_index(theme: i32) -> slint::language::ColorScheme {
    match theme {
        1 => slint::language::ColorScheme::Light,
        2 => slint::language::ColorScheme::Dark,
        _ => slint::language::ColorScheme::Unknown,
    }
}

fn words(text: &[[&str; 4]], lang: i32) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        text.iter()
            .map(|row| SharedString::from(row[lang.clamp(0, 3) as usize]))
            .collect::<Vec<_>>(),
    ))
    .into()
}

fn localize(app: &AppWindow, settings: &SettingsWindow) {
    let lang = app.get_language_index();
    let base = words(i18n::TEXT, lang);
    let v14 = words(i18n::V14, lang);
    let gui = words(i18n::GUI, lang);
    app.set_words(base.clone());
    app.set_v14_words(v14.clone());
    app.set_labels(gui.clone());
    settings.set_words(base);
    settings.set_v14_words(v14);
    settings.set_labels(gui);
    app.set_product_title(format!("IMB-C · {}", i18n::text(lang, 0)).into());
    settings.set_runtime_info(runtime_info(lang).into());
    let theme = palette_index(app.get_theme_index());
    app.global::<Palette>().set_color_scheme(theme);
    settings.global::<Palette>().set_color_scheme(theme);
}

pub(crate) fn runtime_info(lang: i32) -> String {
    let engine = engine::locate_engine();
    let state = if engine.is_file() {
        engine::display_path(&engine)
    } else {
        i18n::gui(lang, 43).to_owned()
    };
    format!(
        "IMB-C v{}\nSlint 1.18.1 · Winit\n{}\nImageMagick 7.1.2-35 · libjxl 0.13.0\nMicrosoft Visual C++ Runtime 14.52.37029.0\n{}\n{state}",
        crate::VERSION,
        i18n::gui(lang, 41),
        i18n::gui(lang, 42),
    )
}
