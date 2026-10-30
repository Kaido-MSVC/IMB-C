#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod i18n;
mod imports;
mod preferences;
use rayon::prelude::*;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

slint::slint! {
    import { VerticalBox, HorizontalBox, Button, ComboBox, ProgressIndicator, TextEdit, AboutSlint, CheckBox, ScrollView, SpinBox } from "std-widgets.slint";
    export { Palette } from "std-widgets.slint";
    component FormatPicker inherits Rectangle {
        in property <bool> enabled: true;
        in-out property <int> current-index: 0;
        callback selected();
        property <[string]> formats: ["WebP", "PNG", "JPEG", "AVIF", "PSD", "TIFF", "JXL"];
        width: 180px; height: 34px; vertical-stretch: 0;
        horizontal-stretch: 0;
        Button { width: 100%; height: 100%; text: root.formats[root.current-index] + "  ▾"; enabled: root.enabled; clicked => { choices.show(); } }
        choices := PopupWindow {
            y: root.height; width: root.width; height: 154px;
            close-policy: close-on-click-outside;
            VerticalBox {
                padding: 5px;
                ScrollView {
                    horizontal-scrollbar-policy: always-off;
                    content-width: self.visible-width;
                    content-height: root.formats.length * 36px;
                    VerticalLayout {
                        spacing: 0px;
                        for item[index] in root.formats: Button {
                            text: item; height: 36px;
                            clicked => { root.current-index = index; root.selected(); choices.close(); }
                        }
                    }
                }
            }
        }
    }
    export component JxlWindow inherits Window {
        in property <[string]> words;
        title: "IMB-C · JPEG ↔ JXL";
        icon: @image-url("../assets/app.png");
        preferred-width: 760px; preferred-height: 470px; min-width: 700px; min-height: 440px;
        in-out property <bool> busy: false;
        in-out property <int> mode: 0;
        in-out property <string> input-path;
        in-out property <string> output-path;
        in-out property <string> reference-path;
        in-out property <string> result;
        callback choose-input(); callback choose-output(); callback choose-reference();
        callback start(); callback cancel();
        VerticalBox {
            padding: 20px; spacing: 12px;
            Text { text: root.words[38]; wrap: word-wrap; }
            ComboBox { model: [root.words[39], root.words[40]]; current-index <=> root.mode; enabled: !root.busy; }
            HorizontalBox {
                Button { text: root.words[7]; enabled: !root.busy; clicked => { root.choose-input(); } }
                Text { text: root.input-path; overflow: elide; }
            }
            HorizontalBox {
                Button { text: root.words[8]; enabled: !root.busy; clicked => { root.choose-output(); } }
                Text { text: root.output-path; overflow: elide; }
            }
            if root.mode == 1: HorizontalBox {
                Button { text: root.words[41]; enabled: !root.busy; clicked => { root.choose-reference(); } }
                Text { text: root.reference-path; overflow: elide; }
            }
            HorizontalBox {
                Button { text: root.words[10]; enabled: !root.busy && root.input-path != "" && root.output-path != ""; clicked => { root.start(); } }
                Button { text: root.words[11]; enabled: root.busy; clicked => { root.cancel(); } }
            }
            TextEdit { text: root.result; read-only: true; vertical-stretch: 1; }
        }
    }
    export component HelpWindow inherits Window {
        in property <string> heading;
        in property <string> content;
        title: "IMB-C · " + root.heading;
        icon: @image-url("../assets/app.png");
        preferred-width: 740px; preferred-height: 620px;
        min-width: 600px; min-height: 440px;
        VerticalBox {
            padding: 20px; spacing: 12px;
            Text { text: root.heading; font-size: 21px; }
            TextEdit { text: root.content; read-only: true; vertical-stretch: 1; }
        }
    }
    export component AboutWindow inherits Window {
        in property <[string]> words;
        in property <string> product-title;
        title: root.product-title;
        icon: @image-url("../assets/app.png");
        width: 680px; height: 600px;
        in property <string> third-party-notices;
        VerticalBox {
            Text { text: root.product-title; font-size: 21px; }
            Text { text: root.words[2] + "\n" + root.words[12]; wrap: word-wrap; }
            AboutSlint { }
            TextEdit { text: root.third-party-notices; read-only: true; vertical-stretch: 1; }
        }
    }
    export component AppWindow inherits Window {
        in property <[string]> words;
        in property <string> product-title;
        title: root.product-title;
        icon: @image-url("../assets/app.png");
        preferred-width: 840px; preferred-height: 820px; min-width: 800px; min-height: 780px;
        in-out property <bool> busy: false;
        in-out property <bool> ready: false;
        in-out property <bool> recursive: false;
        in-out property <int> selected-count: 0;
        in-out property <string> selection: root.selected-count == 0 ? root.words[13] : root.words[25] + " " + root.selected-count + " " + root.words[26];
        in-out property <string> output-path;
        in-out property <string> destination: root.output-path == "" ? root.words[14] : root.output-path;
        in-out property <int> phase: 0;
        in-out property <int> successes: 0;
        in-out property <int> failures: 0;
        in-out property <int> cancelled: 0;
        in-out property <string> status: root.phase == 1 ? root.words[16] : root.phase == 2 ? root.words[17] : root.phase == 5 ? root.words[23] : root.phase >= 3 ? (root.phase == 4 ? root.words[18] : root.words[19]) + ": " + root.words[20] + " " + root.successes + ", " + root.words[21] + " " + root.failures + ", " + root.words[34] + " " + root.cancelled : root.words[15];
        in-out property <int> language-index;
        in-out property <int> theme-index;
        in-out property <bool> save-failed;
        callback preferences-changed();
        in-out property <string> log;
        in-out property <float> progress: 0;
        in-out property <int> format-index: 0;
        in-out property <string> source-folder;
        in-out property <int> avif-quality: 90;
        in-out property <bool> webp-lossless: true;
        in-out property <bool> notify: true;
        in-out property <string> target-icc;
        callback choose-icc();

        MenuBar { Menu { title: root.words[3]; MenuItem { title: root.words[3]; activated => { root.help(); } } MenuItem { title: root.words[4]; activated => { root.about(); } } } }
        callback choose-files(); callback import-folder(); callback jxl(); callback choose-folder(); callback start(); callback cancel(); callback about(); callback help();
        VerticalBox {
            padding: 24px; spacing: 14px;
            HorizontalBox {
                Image { source: @image-url("../assets/app.png"); width: 44px; height: 44px; }
                Text { text: "IMB-C"; font-size: 28px; font-weight: 700; }
                Button { text: root.words[37]; enabled: !root.busy; clicked => { root.jxl(); } }
            }
            HorizontalBox {
                Text { text: root.words[27]; vertical-alignment: center; }
                ComboBox { model: ["繁體中文", "简体中文", "English", "日本語"]; current-index <=> root.language-index; enabled: !root.busy; selected => { root.preferences-changed(); } }
                Text { text: root.words[28]; vertical-alignment: center; }
                ComboBox { model: [root.words[29], root.words[30], root.words[31]]; current-index <=> root.theme-index; selected => { root.preferences-changed(); } }
            }
            if root.save-failed: Text { text: root.words[32]; wrap: word-wrap; }
            HorizontalBox {
                Button { text: root.words[7]; enabled: !root.busy; clicked => { root.choose-files(); } }
                Button { text: root.words[35]; enabled: !root.busy; clicked => { root.import-folder(); } }
                CheckBox { text: root.words[36]; checked <=> root.recursive; enabled: !root.busy; }
                Text { text: root.selection; overflow: elide; }
            }
            HorizontalBox {
                Button { text: root.words[8]; enabled: !root.busy; clicked => { root.choose-folder(); } }
                Text { text: root.destination; overflow: elide; }
            }
            HorizontalBox {
                Text { text: root.words[9]; vertical-alignment: center; }
                format := FormatPicker { enabled: !root.busy; current-index <=> root.format-index; selected => { root.preferences-changed(); } }
                Button { text: root.words[10]; enabled: !root.busy && root.ready; clicked => { root.start(); } }
                Button { text: root.words[11]; enabled: root.busy; clicked => { root.cancel(); } }
            }
            HorizontalBox {
                if root.format-index == 3: Text { text: root.words[42]; vertical-alignment: center; }
                if root.format-index == 3: SpinBox { minimum: 1; maximum: 100; value <=> root.avif-quality; enabled: !root.busy; edited => { root.preferences-changed(); } }
                if root.format-index == 0: CheckBox { text: root.words[43]; checked <=> root.webp-lossless; enabled: !root.busy; toggled => { root.preferences-changed(); } }
                CheckBox { text: root.words[44]; checked <=> root.notify; toggled => { root.preferences-changed(); } }
            }
            HorizontalBox {
                Button { text: root.words[45]; enabled: !root.busy; clicked => { root.choose-icc(); } }
                Text { text: root.target-icc == "" ? root.words[48] : root.target-icc; overflow: elide; }
            }
            ProgressIndicator { progress: root.progress; }
            Text { text: root.status; wrap: word-wrap; }
            TextEdit { text: root.log; read-only: true; vertical-stretch: 1; }
        }
    }
}

#[derive(Default)]
struct Selection {
    files: Vec<PathBuf>,
    output: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppWindow::new()?;
    let about = AboutWindow::new()?;
    let help = HelpWindow::new()?;
    let jxl = JxlWindow::new()?;
    let prefs_path = preferences::path()?;
    let prefs = if prefs_path.is_file() {
        preferences::load(&prefs_path)
    } else {
        preferences::load(&std::env::current_exe()?.with_file_name("imb-c-ui.ini"))
    };
    app.set_format_index(prefs.format);
    app.set_source_folder(prefs.source_folder.to_string_lossy().into_owned().into());
    if prefs.output_folder.is_dir() {
        app.set_output_path(prefs.output_folder.to_string_lossy().into_owned().into());
    }
    app.set_avif_quality(prefs.avif_quality);
    app.set_webp_lossless(prefs.webp_lossless);
    app.set_notify(prefs.notify);
    app.set_language_index(prefs.language);
    app.set_theme_index(prefs.theme);
    apply_preferences(&app, &about, &help);
    about.set_third_party_notices(include_str!("../THIRD-PARTY-RUNTIME.txt").into());
    let weak = app.as_weak();
    let about_weak = about.as_weak();
    let help_weak = help.as_weak();
    app.on_preferences_changed(move || {
        let a = weak.unwrap();
        if let (Some(b), Some(h)) = (about_weak.upgrade(), help_weak.upgrade()) {
            apply_preferences(&a, &b, &h);
        }
        a.set_save_failed(
            preferences::save(
                &prefs_path,
                preferences::Preferences {
                    language: a.get_language_index(),
                    theme: a.get_theme_index(),
                    format: a.get_format_index(),
                    source_folder: PathBuf::from(a.get_source_folder().as_str()),
                    output_folder: PathBuf::from(a.get_output_path().as_str()),
                    avif_quality: a.get_avif_quality(),
                    webp_lossless: a.get_webp_lossless(),
                    notify: a.get_notify(),
                },
            )
            .is_err(),
        );
    });
    let engine = engine::locate_engine();
    let state = Rc::new(RefCell::new(Selection {
        files: Vec::new(),
        output: if prefs.output_folder.is_dir() {
            Some(prefs.output_folder.clone())
        } else {
            None
        },
    }));
    let weak = app.as_weak();
    app.on_choose_icc(move || {
        let a = weak.unwrap();
        // Cancelling clears a previously selected target profile.
        let picked = rfd::FileDialog::new()
            .add_filter("ICC", &["icc", "icm"])
            .pick_file();
        a.set_target_icc(
            picked
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
        );
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = Rc::new(RefCell::new(None::<std::thread::JoinHandle<()>>));
    let weak = app.as_weak();
    let s = state.clone();
    app.on_choose_files(move || {
        if let Some(files) = folder_dialog(weak.unwrap().get_source_folder().as_str())
            .add_filter(
                i18n::text(weak.unwrap().get_language_index(), 24),
                &[
                    "webp", "png", "jpg", "jpeg", "avif", "psd", "tif", "tiff", "jxl",
                ],
            )
            .pick_files()
        {
            let a = weak.unwrap();
            let mut s = s.borrow_mut();
            if let Some(parent) = files.first().and_then(|p| p.parent()) {
                a.set_source_folder(parent.to_string_lossy().into_owned().into());
                a.invoke_preferences_changed();
            }
            s.files = files;
            a.set_selected_count(s.files.len() as i32);
            a.set_log(
                s.files
                    .iter()
                    .take(200)
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
            );
            a.set_ready(!s.files.is_empty() && s.output.is_some());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_choose_folder(move || {
        if let Some(folder) = folder_dialog(weak.unwrap().get_output_path().as_str()).pick_folder()
        {
            let a = weak.unwrap();
            let mut s = s.borrow_mut();
            a.set_output_path(folder.display().to_string().into());
            s.output = Some(folder);
            a.invoke_preferences_changed();
            a.set_ready(!s.files.is_empty());
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_import_folder(move || {
        let a = weak.unwrap();
        if let Some(folder) = folder_dialog(a.get_source_folder().as_str()).pick_folder() {
            a.set_source_folder(folder.to_string_lossy().into_owned().into());
            a.invoke_preferences_changed();
            match imports::folder(&folder, a.get_recursive()) {
                Ok((files, warnings)) => {
                    let mut s = s.borrow_mut();
                    s.files = files;
                    a.set_selected_count(s.files.len() as i32);
                    let mut log = String::new();
                    for line in s
                        .files
                        .iter()
                        .take(200)
                        .map(|p| p.display().to_string())
                        .chain(warnings)
                    {
                        log = append_log(&log, &line);
                    }
                    a.set_log(log.into());
                    a.set_ready(!s.files.is_empty() && s.output.is_some());
                }
                Err(e) => {
                    a.set_log(e.into());
                }
            }
        }
    });
    let jw = jxl.as_weak();
    let aw = app.as_weak();
    app.on_jxl(move || {
        let a = aw.unwrap();
        let j = jw.unwrap();
        j.set_words(a.get_words());
        j.global::<Palette>()
            .set_color_scheme(a.global::<Palette>().get_color_scheme());
        let _ = j.show();
    });
    let jw = jxl.as_weak();
    jxl.on_choose_input(move || {
        let j = jw.unwrap();
        let exts: &[&str] = if j.get_mode() == 0 {
            &["jpg", "jpeg"]
        } else {
            &["jxl"]
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JPEG / JXL", exts)
            .pick_file()
        {
            j.set_input_path(path.to_string_lossy().into_owned().into());
        }
    });
    let jw = jxl.as_weak();
    jxl.on_choose_output(move || {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            jw.unwrap()
                .set_output_path(path.to_string_lossy().into_owned().into());
        }
    });
    let jw = jxl.as_weak();
    jxl.on_choose_reference(move || {
        // Cancelling this dialog clears an optional reference selected previously.
        let path = rfd::FileDialog::new()
            .add_filter("JPEG", &["jpg", "jpeg"])
            .pick_file();
        jw.unwrap().set_reference_path(
            path.map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
        );
    });
    let c = cancel.clone();
    jxl.on_cancel(move || {
        c.store(true, Ordering::Relaxed);
    });
    let c = cancel.clone();
    jxl.window().on_close_requested(move || {
        c.store(true, Ordering::Relaxed);
        slint::CloseRequestResponse::HideWindow
    });
    let jw = jxl.as_weak();
    let aw = app.as_weak();
    let c = cancel.clone();
    let slot = worker.clone();
    jxl.on_start(move || {
        let j = jw.unwrap();
        let a = aw.unwrap();
        if a.get_busy() || j.get_busy() {
            return;
        }
        if let Some(previous) = slot.borrow_mut().take() {
            let _ = previous.join();
        }
        let input = PathBuf::from(j.get_input_path().as_str());
        let dir = PathBuf::from(j.get_output_path().as_str());
        let reference = if j.get_reference_path().is_empty() {
            None
        } else {
            Some(PathBuf::from(j.get_reference_path().as_str()))
        };
        let wrap = j.get_mode() == 0;
        let tools = std::env::current_exe().unwrap().with_file_name("JXL");
        c.store(false, Ordering::Relaxed);
        j.set_busy(true);
        a.set_busy(true);
        j.set_result(i18n::text(a.get_language_index(), 16).into());
        let jw = jw.clone();
        let aw = aw.clone();
        let c = c.clone();
        *slot.borrow_mut() = Some(std::thread::spawn(move || {
            let result =
                engine::jpeg_reconstruction(&tools, &input, &dir, wrap, reference.as_deref(), &c);
            let text = match result {
                Ok((path, hash, verified)) => format!(
                    "{}\nSHA-256: {}\n{}",
                    path.display(),
                    hash,
                    if verified {
                        "SHA-256 ✓ JPEG"
                    } else {
                        "SHA-256: 原 JPEG 未提供 / No reference JPEG"
                    }
                ),
                Err(e) => e,
            };
            let _ = jw.upgrade_in_event_loop(move |j| {
                j.set_busy(false);
                j.set_result(text.into());
            });
            let _ = aw.upgrade_in_event_loop(move |a| {
                a.set_busy(false);
            });
        }));
    });
    let c = cancel.clone();
    let weak = app.as_weak();
    app.on_cancel(move || {
        c.store(true, Ordering::Relaxed);
        weak.unwrap().set_phase(2);
    });
    let about_weak = about.as_weak();
    let jxl_weak = jxl.as_weak();
    let close_cancel = cancel.clone();
    let help_weak = help.as_weak();
    app.window().on_close_requested(move || {
        close_cancel.store(true, Ordering::Relaxed);
        if let Some(window) = help_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = about_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = jxl_weak.upgrade() {
            let _ = window.hide();
        }
        slint::CloseRequestResponse::HideWindow
    });
    app.on_help(move || {
        let _ = help.show();
    });
    app.on_about(move || {
        let _ = about.show();
    });
    let weak = app.as_weak();
    let c = cancel.clone();
    let worker_slot = worker.clone();
    let jxl_weak = jxl.as_weak();
    app.on_start(move || {
        let a = weak.unwrap();
        if a.get_busy() {
            return;
        }
        if let Some(window) = jxl_weak.upgrade() {
            let _ = window.hide();
        }
        let s = state.borrow();
        let Some(folder) = s.output.clone() else {
            return;
        };
        let files = s.files.clone();
        let format = engine::Format::from_index(a.get_format_index());
        let options = engine::Options {
            avif_quality: a.get_avif_quality().clamp(1, 100) as u8,
            webp_lossless: a.get_webp_lossless(),
            target_icc: if a.get_target_icc().is_empty() {
                None
            } else {
                Some(PathBuf::from(a.get_target_icc().as_str()))
            },
        };
        let notify = a.get_notify();
        a.set_busy(true);
        a.set_progress(0.0);
        a.set_log("".into());
        a.set_successes(0);
        a.set_failures(0);
        a.set_cancelled(0);
        a.set_phase(1);
        let lang = a.get_language_index();
        c.store(false, Ordering::Relaxed);
        let weak = weak.clone();
        let c = c.clone();
        let engine = engine.clone();
        if let Some(previous) = worker_slot.borrow_mut().take() {
            let _ = previous.join();
        }
        let handle = std::thread::spawn(move || {
            let done = AtomicUsize::new(0);
            let failed = AtomicUsize::new(0);
            let cancelled = AtomicUsize::new(0);
            let workers = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(2);
            let result = rayon::ThreadPoolBuilder::new().num_threads(workers).build();
            if let Ok(pool) = result {
                pool.install(|| {
                    files.par_iter().for_each(|input| {
                        let result = engine::convert_with_options(
                            &engine, input, &folder, format, &options, &c,
                        );
                        let line = match result {
                            Ok((p, verification)) => format!(
                                "{}: {} → {}\n  {}",
                                i18n::text(lang, 22),
                                input.display(),
                                p.display(),
                                verification
                            ),
                            Err(e) if e == "已取消" => {
                                cancelled.fetch_add(1, Ordering::Relaxed);
                                done.fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                            Err(e) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                                format!("{}: {} — {}", i18n::text(lang, 21), input.display(), e)
                            }
                        };
                        done.fetch_add(1, Ordering::Relaxed);
                        let total = files.len();
                        let _ = weak.upgrade_in_event_loop(move |a| {
                            a.set_log(append_log(&a.get_log(), &line).into());
                            a.set_progress((a.get_progress() + 1.0 / total.max(1) as f32).min(1.0));
                        });
                    })
                });
            } else {
                let _ = weak.upgrade_in_event_loop(move |a| {
                    a.set_log(i18n::text(lang, 23).into());
                    a.set_busy(false);
                    a.set_phase(5);
                });
                return;
            }
            let count = done.load(Ordering::Relaxed);
            let failures = failed.load(Ordering::Relaxed);
            let cancellations = cancelled.load(Ordering::Relaxed);
            let stopped = c.load(Ordering::Relaxed);
            let _ = weak.upgrade_in_event_loop(move |a| {
                a.set_busy(false);
                a.set_successes((count - failures - cancellations) as i32);
                a.set_cancelled(cancellations as i32);
                a.set_progress(count as f32 / files.len().max(1) as f32);
                a.set_failures(failures as i32);
                a.set_phase(if stopped { 4 } else { 3 });
                if notify && !stopped {
                    rfd::MessageDialog::new()
                        .set_title("IMB-C")
                        .set_description(format!(
                            "{}: {} {}, {} {}",
                            i18n::text(lang, 19),
                            i18n::text(lang, 20),
                            count - failures - cancellations,
                            i18n::text(lang, 21),
                            failures
                        ))
                        .show();
                }
            });
        });
        *worker_slot.borrow_mut() = Some(handle);
    });
    let ui_result = app.run();
    cancel.store(true, Ordering::Relaxed);
    // Workers observe cancellation, terminate their ImageMagick child, and remove temporary files.
    if let Some(handle) = worker.borrow_mut().take() {
        let _ = handle.join();
    }
    ui_result?;
    Ok(())
}

fn apply_preferences(app: &AppWindow, about: &AboutWindow, help: &HelpWindow) {
    let lang = app.get_language_index();
    let words: slint::ModelRc<slint::SharedString> = Rc::new(slint::VecModel::from(
        i18n::TEXT
            .iter()
            .map(|row| slint::SharedString::from(row[lang.clamp(0, 3) as usize]))
            .collect::<Vec<_>>(),
    ))
    .into();
    app.set_words(words.clone());
    about.set_words(words);
    help.set_heading(i18n::text(lang, 3).into());
    help.set_content(
        format!(
            "{}\n{}\n\n{}\n{}\n{}\n\n{}\n{}\n\n{}\nImageMagick: {}\n{}",
            i18n::text(lang, 49),
            i18n::text(lang, 6),
            i18n::text(lang, 50),
            i18n::text(lang, 47),
            i18n::text(lang, 12),
            i18n::text(lang, 51),
            i18n::text(lang, 46),
            i18n::text(lang, 52),
            engine::locate_engine().display(),
            i18n::text(lang, 33),
        )
        .into(),
    );
    app.set_product_title(
        format!(
            "IMB-C v{VERSION} · {} · {}",
            i18n::text(lang, 0),
            i18n::text(lang, 1)
        )
        .into(),
    );
    about.set_product_title(format!("IMB-C v{VERSION} · {}", i18n::text(lang, 1)).into());
    let scheme = match app.get_theme_index() {
        1 => slint::language::ColorScheme::Light,
        2 => slint::language::ColorScheme::Dark,
        _ => slint::language::ColorScheme::Unknown,
    };
    app.global::<Palette>().set_color_scheme(scheme);
    about.global::<Palette>().set_color_scheme(scheme);
    help.global::<Palette>().set_color_scheme(scheme);
}

// Retain at most 200 lines and 64 KiB of recent text.
fn append_log(previous: &str, line: &str) -> String {
    let combined = format!("{previous}{line}\n");
    let mut bytes = 0;
    let mut lines = Vec::new();
    for line in combined.lines().rev().take(200) {
        if bytes + line.len() + 1 > 65536 {
            break;
        }
        bytes += line.len() + 1;
        lines.push(line);
    }
    lines.reverse();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn folder_dialog(path: &str) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    if std::path::Path::new(path).is_dir() {
        dialog.set_directory(path)
    } else {
        dialog
    }
}
