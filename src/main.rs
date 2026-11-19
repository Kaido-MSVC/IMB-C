#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod batch_warnings;
mod cli;
mod engine;
mod i18n;
mod iat;
mod imports;
mod preferences;
mod ui_queue;
mod ui_settings;
mod validation_queue;
use rayon::prelude::*;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
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
    export { Palette } from "std-widgets.slint";
    export { AppWindow } from "ui/app.slint";
    export { SettingsWindow } from "ui/settings.slint";
    export { JxlWindow, WarningWindow, CompletionWindow, HelpWindow, AboutWindow } from "ui/auxiliary.slint";
}

/// 進行中的背景工作數（轉檔批次、JPEG ↔ JXL 工具）；結束看門狗在結束程序前等待歸零。
static ACTIVE_WORK: AtomicUsize = AtomicUsize::new(0);
/// `app.run()` 已返回：主執行緒會自行收尾，看門狗不介入。
static EVENT_LOOP_RETURNED: AtomicBool = AtomicBool::new(false);
static UI_CLOSING: AtomicBool = AtomicBool::new(false);
struct WorkGuard;
impl WorkGuard {
    /// 在建立執行緒**之前**計入，避免看門狗在執行緒開始前看到 0。
    fn new() -> Self {
        ACTIVE_WORK.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for WorkGuard {
    fn drop(&mut self) {
        ACTIVE_WORK.fetch_sub(1, Ordering::SeqCst);
    }
}
/// 主視窗已要求關閉，但事件迴圈 5 秒內沒有結束時的保險（RC2 第三輪 M3）。
///
/// 實測：外部工具若先對 winit 內部的「Winit Thread Event Target」視窗送 WM_CLOSE，該視窗被銷毀後，
/// 視窗計數歸零或 `quit_event_loop` 的結束訊息都無法送達，事件迴圈會在沒有任何可見視窗的情況下持續執行。
/// 看門狗等待背景工作完成收尾。即使同步 I/O 仍在等待，也不強制中斷寫入或帳本更新。
fn start_exit_watchdog() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if EVENT_LOOP_RETURNED.load(Ordering::SeqCst) {
            return;
        }
        while ACTIVE_WORK.load(Ordering::SeqCst) > 0 {
            if EVENT_LOOP_RETURNED.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if EVENT_LOOP_RETURNED.load(Ordering::SeqCst) {
            return;
        }
        engine::shutdown_instance();
        std::process::exit(0);
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    engine::initialize_dll_search()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if cli::is_cli_invocation(&args) {
        std::process::exit(cli::run_from_gui(args));
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;
    let app = AppWindow::new()?;
    let settings = SettingsWindow::new()?;
    let diagnostics = HelpWindow::new()?;
    let about = AboutWindow::new()?;
    let help = HelpWindow::new()?;
    let jxl = JxlWindow::new()?;
    let warnings_window = WarningWindow::new()?;
    let completion = CompletionWindow::new()?;
    // Internal report path for file operations (may be verbatim or longer than 260
    // characters); the `report-path` properties only hold the display_path text.
    let report_file = ReportFile::default();
    let cw = completion.as_weak();
    completion.on_dismiss(move || {
        let _ = cw.unwrap().hide();
    });
    let cw = completion.as_weak();
    let report = report_file.clone();
    completion.on_open_report(move || {
        open_report(report.get(), &cw.unwrap().get_words());
    });
    let ww = warnings_window.as_weak();
    let report = report_file.clone();
    warnings_window.on_open_report(move || {
        open_report(report.get(), &ww.unwrap().get_words());
    });
    let ww = warnings_window.as_weak();
    let aw = app.as_weak();
    let report = report_file.clone();
    app.on_show_warnings(move || {
        let a = aw.unwrap();
        let w = ww.unwrap();
        w.set_words(a.get_words());
        w.set_summary(a.get_batch_summary());
        w.set_report_error(a.get_warning_report_error());
        w.set_report_path(a.get_warning_report());
        load_warning_page(&w, report.get().as_deref(), 0);
        let _ = w.show();
    });
    let ww = warnings_window.as_weak();
    let report = report_file.clone();
    warnings_window.on_previous(move || {
        let w = ww.unwrap();
        let page = w.get_page().saturating_sub(1).max(0);
        load_warning_page(&w, report.get().as_deref(), page);
    });
    let ww = warnings_window.as_weak();
    let report = report_file.clone();
    warnings_window.on_next(move || {
        let w = ww.unwrap();
        let page = w.get_page().saturating_add(1);
        load_warning_page(&w, report.get().as_deref(), page);
    });
    let prefs_path = preferences::path()?;
    let prefs = if prefs_path.is_file() {
        preferences::load(&prefs_path)
    } else {
        preferences::load(&std::env::current_exe()?.with_file_name("imb-c-ui.ini"))
    };
    let prefs = Rc::new(RefCell::new(prefs));
    let bw = about.as_weak();
    let hw = help.as_weak();
    let jw = jxl.as_weak();
    let ww = warnings_window.as_weak();
    let cw = completion.as_weak();
    let dw = diagnostics.as_weak();
    let settings_probe = ui_settings::connect(&app, &settings, &prefs_path, &prefs, move |a| {
        let (b, h, j, w, c, d) = (
            bw.unwrap(),
            hw.unwrap(),
            jw.unwrap(),
            ww.unwrap(),
            cw.unwrap(),
            dw.unwrap(),
        );
        apply_auxiliary_text(a, &b, &h);
        let words = a.get_words();
        j.set_words(words.clone());
        w.set_words(words.clone());
        c.set_words(words);
        d.set_heading(i18n::text(a.get_language_index(), 52).into());
        let theme = ui_settings::palette_index(a.get_theme_index());
        b.global::<Palette>().set_color_scheme(theme);
        h.global::<Palette>().set_color_scheme(theme);
        j.global::<Palette>().set_color_scheme(theme);
        w.global::<Palette>().set_color_scheme(theme);
        c.global::<Palette>().set_color_scheme(theme);
        d.global::<Palette>().set_color_scheme(theme);
    });
    about.set_third_party_notices(include_str!("../THIRD-PARTY-RUNTIME.txt").into());

    let aw = app.as_weak();
    let dw = diagnostics.as_weak();
    settings.on_show_diagnostics(move || {
        let a = aw.unwrap();
        let d = dw.unwrap();
        let lang = a.get_language_index();
        d.set_heading(i18n::text(lang, 52).into());
        d.set_content(
            format!(
                "{}\n\n{}\n{}\n{}\n\n{}\n{}\n{}",
                ui_settings::runtime_info(lang),
                i18n::gui(lang, 47),
                engine::display_path(&prefs_path),
                i18n::gui(lang, 48),
                i18n::text(lang, 62),
                a.get_warning_report(),
                a.get_log(),
            )
            .into(),
        );
        let _ = d.show();
    });
    let engine = engine::locate_engine();
    // 背景回收先前異常結束留下的暫存（只處理已終止執行個體的擁有權紀錄）；有結果才寫入日誌。
    let recovery_weak = app.as_weak();
    std::thread::spawn(move || {
        let lines = engine::recover_stale_temp();
        if !lines.is_empty() {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(a) = recovery_weak.upgrade() {
                    let mut log = a.get_log().to_string();
                    for line in &lines {
                        log = append_log(&log, line);
                    }
                    a.set_log(log.into());
                }
            });
        }
    });
    let files = ui_queue::connect(&app, &prefs);
    let target_profile = Rc::new(RefCell::new(None::<PathBuf>));
    let weak = app.as_weak();
    let profile = target_profile.clone();
    app.on_choose_icc(move || {
        let a = weak.unwrap();
        if let Some(picked) = a
            .window()
            .with_winit_window(|parent| {
                rfd::FileDialog::new()
                    .set_parent(parent)
                    .add_filter("ICC", &["icc", "icm"])
                    .pick_file()
            })
            .flatten()
        {
            a.set_target_icc(engine::display_path(&picked).into());
            *profile.borrow_mut() = Some(picked);
        }
    });
    let weak = app.as_weak();
    let profile = target_profile.clone();
    app.on_clear_icc(move || {
        *profile.borrow_mut() = None;
        weak.unwrap().set_target_icc("".into());
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = Rc::new(RefCell::new(None::<std::thread::JoinHandle<()>>));
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
    let jw = jxl.as_weak();
    jxl.window().on_close_requested(move || {
        if jw.upgrade().is_some_and(|j| j.get_busy()) {
            c.store(true, Ordering::Relaxed);
        }
        slint::CloseRequestResponse::HideWindow
    });
    let jw = jxl.as_weak();
    let aw = app.as_weak();
    let c = cancel.clone();
    let slot = worker.clone();
    let sw = settings.as_weak();
    jxl.on_start(move || {
        let j = jw.unwrap();
        let a = aw.unwrap();
        if UI_CLOSING.load(Ordering::SeqCst) || a.get_busy() || j.get_busy() {
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
        let tools = match engine::locate_jxl_tools(std::env::current_exe()) {
            Ok(tools) => tools,
            Err(e) => {
                j.set_result(e.into());
                return;
            }
        };
        c.store(false, Ordering::Relaxed);
        j.set_busy(true);
        a.set_busy(true);
        a.set_phase(1);
        a.set_batch_total(0);
        a.set_progress(0.0);
        a.set_completed_count(0);
        a.set_validation_count(0);
        a.set_validation_queued(0);
        a.set_active_stage("JPEG ↔ JXL".into());
        sw.unwrap().set_busy(true);
        j.set_result(i18n::text(a.get_language_index(), 16).into());
        let jw = jw.clone();
        let aw = aw.clone();
        let sw = sw.clone();
        let c = c.clone();
        let work = WorkGuard::new();
        *slot.borrow_mut() = Some(std::thread::spawn(move || {
            let _work = work;
            let result = engine::jpeg_reconstruction_with_warnings(
                &tools,
                &input,
                &dir,
                wrap,
                reference.as_deref(),
                &c,
            );
            let text = match result {
                Ok((path, hash, verified, warnings)) => format!(
                    "{}\nSHA-256: {}\n{}{}",
                    engine::display_path(&path),
                    hash,
                    if verified {
                        "SHA-256 matches original JPEG"
                    } else {
                        "SHA-256: 原 JPEG 未提供 / No reference JPEG"
                    },
                    warnings
                        .into_iter()
                        .map(|w| format!("\n警告：{w}"))
                        .collect::<String>()
                ),
                Err(e) => e,
            };
            let _ = jw.upgrade_in_event_loop(move |j| {
                j.set_busy(false);
                j.set_result(text.into());
            });
            let _ = aw.upgrade_in_event_loop(move |a| {
                a.set_busy(false);
                a.set_phase(0);
                a.set_active_stage("".into());
                if let Some(w) = sw.upgrade() {
                    w.set_busy(false);
                }
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
    let warnings_weak = warnings_window.as_weak();
    let close_cancel = cancel.clone();
    let help_weak = help.as_weak();
    let completion_close = completion.as_weak();
    let settings_close = settings.as_weak();
    let diagnostics_close = diagnostics.as_weak();
    let closing_probe = settings_probe.clone();
    app.window().on_close_requested(move || {
        UI_CLOSING.store(true, Ordering::SeqCst);
        close_cancel.store(true, Ordering::Relaxed);
        closing_probe.cancel();
        if let Some(window) = settings_close.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = diagnostics_close.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = completion_close.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = warnings_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = help_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = about_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = jxl_weak.upgrade() {
            let _ = window.hide();
        }
        // 關閉主視窗＝結束程式：明確結束事件迴圈，不依賴「最後一個可見視窗」計數
        // （RC2 第三輪 M3：完成通知顯示時經自動化關閉，曾出現所有視窗隱藏但事件迴圈仍執行）。
        let _ = slint::quit_event_loop();
        start_exit_watchdog();
        slint::CloseRequestResponse::HideWindow
    });
    let hw = help.as_weak();
    settings.on_show_help(move || {
        let _ = hw.unwrap().show();
    });
    let bw = about.as_weak();
    app.on_show_about(move || {
        let _ = bw.unwrap().show();
    });
    let bw = about.as_weak();
    settings.on_show_licenses(move || {
        let _ = bw.unwrap().show();
    });
    let weak = app.as_weak();
    let c = cancel.clone();
    let worker_slot = worker.clone();
    let jxl_weak = jxl.as_weak();
    let warnings_weak = warnings_window.as_weak();
    let completion_weak = completion.as_weak();
    let start_report = report_file.clone();
    let sw = settings.as_weak();
    let manual_probe = settings_probe.clone();
    app.on_start(move || {
        let a = weak.unwrap();
        if UI_CLOSING.load(Ordering::SeqCst) || a.get_busy() {
            return;
        }
        if let Some(window) = jxl_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = completion_weak.upgrade() {
            let _ = window.hide();
        }
        if let Some(window) = warnings_weak.upgrade() {
            let _ = window.hide();
        }
        let completion_weak = completion_weak.clone();
        let report_file = start_report.clone();
        let folder = prefs.borrow().output_folder.clone();
        let files = files.borrow().clone();
        if files.is_empty() {
            return;
        }
        if folder.as_os_str().is_empty() || !folder.is_dir() {
            let text = i18n::gui(a.get_language_index(), 39);
            a.set_log(append_log(&a.get_log(), text).into());
            a.set_show_log(true);
            a.set_ready(false);
            return;
        }
        let format = engine::Format::from_index(a.get_format_index());
        let options = engine::Options {
            avif_quality: a.get_avif_quality().clamp(1, 100) as u8,
            avif_lossless: a.get_avif_lossless(),
            jxl_quality: a.get_jxl_quality().clamp(1, 99) as u8,
            jxl_lossless: a.get_jxl_lossless(),
            webp_lossless: a.get_webp_lossless(),
            target_icc: target_profile.borrow().clone(),
            ..Default::default()
        };
        let (notify, iat_enabled, iat_path) = {
            let settings = prefs.borrow();
            (
                settings.notify,
                settings.iat_enabled,
                settings.iat_path.clone(),
            )
        };
        if iat_enabled {
            // 批次會自行檢查版本，避免舊的手動探測結果稍後覆寫批次狀態。
            manual_probe.cancel();
        }
        a.set_busy(true);
        a.set_batch_total(files.len().min(i32::MAX as usize) as i32);
        a.set_dragging(false);
        a.set_show_log(true);
        sw.unwrap().set_busy(true);
        let sw = sw.clone();
        a.set_progress(0.0);
        a.set_completed_count(0);
        a.set_active_stage("".into());
        a.set_log("".into());
        a.set_successes(0);
        a.set_warning_files(0);
        a.set_warning_count(0);
        a.set_warning_report(report_file.set(None).into());
        a.set_warning_report_error("".into());
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
        a.set_validation_count(0);
        a.set_validation_queued(0);
        let work = WorkGuard::new();
        let handle = std::thread::spawn(move || {
            let _work = work;
            let external = if iat_enabled {
                match std::env::current_exe()
                    .map_err(|e| e.to_string())
                    .and_then(|exe| iat::Client::probe(Some(&iat_path), &exe, &c))
                {
                    Ok((client, version)) => {
                        let _ = sw.upgrade_in_event_loop(move |w| {
                            ui_settings::set_probe_status(&w, Ok(version));
                        });
                        Some(client)
                    }
                    Err(_) if c.load(Ordering::Relaxed) => None,
                    Err(e) => {
                        let line = format!("{}（{e}）", i18n::v14(lang, 15));
                        let _ = sw.upgrade_in_event_loop(move |w| {
                            ui_settings::set_probe_status(&w, Err(e));
                        });
                        let _ = weak.upgrade_in_event_loop(move |a| {
                            a.set_log(append_log(&a.get_log(), &line).into())
                        });
                        None
                    }
                }
            } else {
                None
            };
            let warning_journal =
                Arc::new(std::sync::Mutex::new(batch_warnings::Journal::new(&folder)));
            let done = AtomicUsize::new(0);
            let failed = AtomicUsize::new(0);
            let cancelled = AtomicUsize::new(0);
            let system_stopped = Arc::new(AtomicBool::new(false));
            let validation_submitted = AtomicUsize::new(0);
            type Validation = (PathBuf, PathBuf, Vec<String>);
            let validation_queue = external.map(|client| {
                let journal = warning_journal.clone();
                let stopped = system_stopped.clone();
                let cancel = c.clone();
                let ui = weak.clone();
                validation_queue::Queue::new(8, move |(input, output, mut warnings): Validation| {
                    let result = client.compare(&input, &output, &cancel);
                    let text = result.localized(lang);
                    if !result.is_verified() {
                        warnings.push(text.clone());
                    }
                    if journal.lock().unwrap().record(&input, &warnings).is_err() {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    let line = format!("{}: {text}", engine::display_path(&output));
                    let _ = ui.upgrade_in_event_loop(move |a| {
                        a.set_validation_count(a.get_validation_count() + 1);
                        a.set_validation_queued(
                            a.get_validation_queued().max(a.get_validation_count()),
                        );
                        a.set_log(append_log(&a.get_log(), &line).into());
                    });
                })
            });
            let validation_queue = match validation_queue {
                Some(Ok(queue)) => Some(queue),
                Some(Err(e)) => {
                    let line = format!("{}（{e}）", i18n::v14(lang, 15));
                    let _ = weak.upgrade_in_event_loop(move |a| {
                        a.set_log(append_log(&a.get_log(), &line).into())
                    });
                    None
                }
                None => None,
            };
            let workers = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(2);
            let result = rayon::ThreadPoolBuilder::new().num_threads(workers).build();
            if let Ok(pool) = result {
                pool.install(|| {
                    files.par_iter().for_each(|input| {
                        if system_stopped.load(Ordering::Relaxed) {
                            failed.fetch_add(1, Ordering::Relaxed);
                            done.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        if !c.load(Ordering::Relaxed) {
                            let line = format!(
                                "{}: {}",
                                i18n::text(lang, 16),
                                engine::display_path(input)
                            );
                            let _ = weak.upgrade_in_event_loop(move |a| {
                                a.set_log(append_log(&a.get_log(), &line).into());
                            });
                        }
                        let stage_weak = weak.clone();
                        let name = input
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let result = engine::convert_with_progress(
                            &engine,
                            input,
                            &folder,
                            format,
                            &options,
                            &c,
                            move |stage| {
                                let text =
                                    format!("{}：{name}", i18n::v14(lang, 8 + usize::from(stage)));
                                let _ = stage_weak.upgrade_in_event_loop(move |a| {
                                    a.set_active_stage(text.into())
                                });
                            },
                        );
                        let line = match result {
                            Ok((p, verification)) => {
                                let mut reported_warnings = verification.warnings.clone();
                                let mut record_here = true;
                                let mut external_text = String::new();
                                if let Some(queue) = &validation_queue {
                                    match queue.submit((
                                        input.clone(),
                                        p.clone(),
                                        reported_warnings.clone(),
                                    )) {
                                        Ok(()) => {
                                            let queued = validation_submitted
                                                .fetch_add(1, Ordering::Relaxed)
                                                + 1;
                                            let _ = weak.upgrade_in_event_loop(move |a| {
                                                a.set_validation_queued(
                                                    a.get_validation_queued().max(queued as i32),
                                                )
                                            });
                                            record_here = false;
                                            external_text = format!("\n  {}", i18n::v14(lang, 17));
                                        }
                                        Err(_) => {
                                            let text = i18n::v14(lang, 18).to_owned();
                                            reported_warnings.push(text.clone());
                                            external_text = format!("\n  {text}");
                                        }
                                    }
                                }
                                if record_here
                                    && warning_journal
                                        .lock()
                                        .unwrap()
                                        .record(input, &reported_warnings)
                                        .is_err()
                                {
                                    system_stopped.store(true, Ordering::Relaxed);
                                }
                                format!(
                                    "{}: {} → {}\n  {}{}",
                                    i18n::text(lang, 22),
                                    engine::display_path(input),
                                    engine::display_path(&p),
                                    verification,
                                    external_text
                                )
                            }
                            Err(e) if e == "已取消" => {
                                cancelled.fetch_add(1, Ordering::Relaxed);
                                done.fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                            Err(e) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                                format!(
                                    "{}: {} — {}",
                                    i18n::text(lang, 21),
                                    engine::display_path(input),
                                    engine::localize_color_error(lang, &e)
                                )
                            }
                        };
                        let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let total = files.len();
                        let _ = weak.upgrade_in_event_loop(move |a| {
                            a.set_log(append_log(&a.get_log(), &line).into());
                            a.set_completed_count(a.get_completed_count().max(finished as i32));
                            a.set_progress(
                                a.get_progress()
                                    .max(finished as f32 / total.max(1) as f32)
                                    .min(1.0),
                            );
                        });
                    })
                });
            } else {
                let _ = weak.upgrade_in_event_loop(move |a| {
                    a.set_log(i18n::text(lang, 23).into());
                    a.set_busy(false);
                    a.set_phase(5);
                    if let Some(w) = sw.upgrade() {
                        w.set_busy(false);
                    }
                });
                return;
            }
            if let Some(queue) = validation_queue {
                let _ = weak
                    .upgrade_in_event_loop(move |a| a.set_active_stage(i18n::v14(lang, 13).into()));
                if queue.finish().is_err() {
                    system_stopped.store(true, Ordering::Relaxed);
                    let line = i18n::v14(lang, 18);
                    let _ = weak.upgrade_in_event_loop(move |a| {
                        a.set_log(append_log(&a.get_log(), line).into())
                    });
                }
            }
            let count = done.load(Ordering::Relaxed);
            let failures = failed.load(Ordering::Relaxed);
            let cancellations = cancelled.load(Ordering::Relaxed);
            let warning_summary = warning_journal.lock().unwrap().finish(
                count - failures - cancellations,
                failures,
                cancellations,
            );
            let user_cancelled = c.load(Ordering::Relaxed);
            let system_aborted = system_stopped.load(Ordering::Relaxed);
            let stopped = user_cancelled || system_aborted;
            let _ = weak.upgrade_in_event_loop(move |a| {
                a.set_busy(false);
                if let Some(w) = sw.upgrade() {
                    w.set_busy(false);
                }
                a.set_successes(
                    batch_warnings::clean_successes(
                        count,
                        failures,
                        cancellations,
                        warning_summary.files,
                    )
                    .min(i32::MAX as usize) as i32,
                );
                a.set_warning_files(warning_summary.files.min(i32::MAX as usize) as i32);
                a.set_warning_count(warning_summary.messages.min(i32::MAX as usize) as i32);
                a.set_warning_report(report_file.set(warning_summary.path).into());
                a.set_warning_report_error(warning_summary.error.unwrap_or_default().into());
                a.set_cancelled(cancellations as i32);
                a.set_completed_count(count as i32);
                a.set_progress(count as f32 / files.len().max(1) as f32);
                a.set_failures(failures as i32);
                a.set_phase(if system_aborted {
                    5
                } else if user_cancelled {
                    4
                } else {
                    3
                });
                if notify
                    && !stopped
                    && !UI_CLOSING.load(Ordering::SeqCst)
                    && let Some(w) = completion_weak.upgrade()
                {
                    w.set_words(a.get_words());
                    w.set_status(a.get_batch_summary());
                    w.set_report_path(a.get_warning_report());
                    w.global::<Palette>()
                        .set_color_scheme(a.global::<Palette>().get_color_scheme());
                    let _ = w.show();
                }
            });
        });
        *worker_slot.borrow_mut() = Some(handle);
    });
    let ui_result = app.run();
    EVENT_LOOP_RETURNED.store(true, Ordering::SeqCst);
    cancel.store(true, Ordering::Relaxed);
    settings_probe.shutdown();
    // Workers observe cancellation, terminate their ImageMagick child, and remove temporary files.
    if let Some(handle) = worker.borrow_mut().take() {
        let _ = handle.join();
    }
    // 所有工作已結束；若沒有保留中的暫存，移除本執行個體的擁有權日誌。
    engine::shutdown_instance();
    ui_result?;
    Ok(())
}

/// The current batch's warning report: the internal path is used to read and open the
/// file, `set` returns the text shown on screen.
#[derive(Clone, Default)]
struct ReportFile(Arc<std::sync::Mutex<Option<PathBuf>>>);
impl ReportFile {
    fn set(&self, path: Option<PathBuf>) -> String {
        let shown = path.as_ref().map(engine::display_path).unwrap_or_default();
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = path;
        shown
    }
    fn get(&self) -> Option<PathBuf> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn open_report(path: Option<PathBuf>, words: &slint::ModelRc<slint::SharedString>) {
    let path = path.unwrap_or_default();
    if let Err(e) = open_with_default_app(&path) {
        let heading = slint::Model::row_data(words, 63).unwrap_or_default();
        rfd::MessageDialog::new()
            .set_title("IMB-C")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{heading}\n{}\n{e}", engine::display_path(&path)))
            .show();
    }
}

fn load_warning_page(window: &WarningWindow, path: Option<&std::path::Path>, page: i32) {
    let Some(path) = path else {
        window.set_content(window.get_report_error());
        window.set_has_previous(false);
        window.set_has_next(false);
        return;
    };
    match batch_warnings::read_page(path, page.max(0) as u64) {
        Ok(data) => {
            let error = window.get_report_error();
            window.set_content(
                if error.is_empty() {
                    data.text
                } else {
                    format!("{error}\n\n{}", data.text)
                }
                .into(),
            );
            window.set_page(page);
            window.set_has_previous(data.previous);
            window.set_has_next(data.next);
        }
        Err(e) => {
            window.set_content(
                format!("無法讀取完整警告報告：{e}\n{}", window.get_report_error()).into(),
            );
            window.set_has_previous(false);
            window.set_has_next(false);
        }
    }
}

fn apply_auxiliary_text(app: &AppWindow, about: &AboutWindow, help: &HelpWindow) {
    let lang = app.get_language_index();
    about.set_words(app.get_words());
    help.set_heading(i18n::text(lang, 3).into());
    help.set_content(
        format!(
            "{}\n{}\n\n{}\n{}\n{}\n{}\n\n{}\n{}\n\n{}\nImageMagick: {}\n{}",
            i18n::text(lang, 49),
            i18n::text(lang, 6),
            i18n::text(lang, 50),
            i18n::text(lang, 47),
            i18n::text(lang, 12),
            i18n::v14(lang, 14),
            i18n::text(lang, 51),
            i18n::text(lang, 46),
            i18n::text(lang, 52),
            engine::display_path(engine::locate_engine()),
            i18n::text(lang, 33),
        )
        .into(),
    );
    about.set_product_title(format!("IMB-C v{VERSION} · {}", i18n::gui(lang, 30)).into());
}

/// Open a file with its associated program; failures are returned, never panicked on.
#[cfg(windows)]
fn open_with_default_app(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    if path.as_os_str().is_empty() || !path.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::NotFound).to_string());
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let operation = wide("open".as_ref());
    let file = wide(path.as_os_str());
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call.
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    if code > 32 {
        Ok(())
    } else {
        let os = std::io::Error::last_os_error();
        Err(match os.raw_os_error() {
            Some(0) | None => format!("ShellExecuteW error {code}"),
            Some(_) => os.to_string(),
        })
    }
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

#[cfg(test)]
mod warning_ui_tests {
    #[test]
    fn report_file_keeps_internal_long_path_and_shows_display_path() {
        let dir = tempfile::tempdir().unwrap();
        // Verbatim, > 260 characters, Chinese and spaces.
        let mut long = std::path::PathBuf::from(format!(r"\\?\{}", dir.path().display()));
        for i in 0..6 {
            long.push(format!("很長的 資料夾 {i} {}", "x".repeat(40)));
        }
        std::fs::create_dir_all(&long).unwrap();
        let mut journal = crate::batch_warnings::Journal::new(&long);
        journal
            .record(std::path::Path::new("a.avif"), &["w".into()])
            .unwrap();
        let internal = journal.finish(1, 0, 0).path.unwrap();
        assert!(internal.as_os_str().len() > 260);
        let report = super::ReportFile::default();
        let shown = report.set(Some(internal.clone()));
        assert_eq!(report.get().as_deref(), Some(internal.as_path()));
        assert!(!shown.starts_with(r"\\?\") && shown == crate::engine::display_path(&internal));
        // File operations use the stored internal path.
        let page = crate::batch_warnings::read_page(&report.get().unwrap(), 0).unwrap();
        assert!(page.text.contains("Source: a.avif"));
        assert_eq!(report.set(None), "");
        assert!(report.get().is_none());
    }
    #[test]
    fn n2_open_report_reports_missing_file_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::open_with_default_app(&dir.path().join("無 報告.txt")).is_err());
        assert!(super::open_with_default_app(std::path::Path::new("")).is_err());
        assert!(super::open_with_default_app(dir.path()).is_err());
    }
    #[test]
    fn final_150_warning_counts_and_report_survive_200_line_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = crate::batch_warnings::Journal::new(dir.path());
        let mut log = String::new();
        for i in 1..=155 {
            let name = format!("artificial-{i:03}.avif");
            log = super::append_log(&log, &format!("Processing: {name}"));
            let warnings = if i <= 150 {
                vec!["AVIF 缺少 ICC／CICP；色彩未驗證；請保留原檔".repeat(3)]
            } else {
                Vec::new()
            };
            journal
                .record(std::path::Path::new(&name), &warnings)
                .unwrap();
            log = super::append_log(&log, &format!("Success: {name}\n  {}", warnings.join("；")));
        }
        let summary = journal.finish(155, 0, 0);
        assert_eq!((summary.files, summary.messages), (150, 150));
        assert_eq!(
            crate::batch_warnings::clean_successes(155, 0, 0, summary.files),
            5
        );
        assert!(log.lines().count() <= 200 && log.len() <= 65536);
        assert!(!log.contains("artificial-001.avif"));
        let report = std::fs::read_to_string(summary.path.as_ref().unwrap()).unwrap();
        assert_eq!(report.matches("Source:").count(), 150);
        assert_eq!(report.matches("Warning:").count(), 150);
        assert!(report.contains("artificial-001.avif") && report.contains("artificial-150.avif"));
        assert!(report.contains(
            "Completed: 155; with warnings: 150; warning messages: 150; failed: 0; cancelled: 0"
        ));
        let mut full = String::new();
        let mut page_index = 0;
        loop {
            let page = crate::batch_warnings::read_page(summary.path.as_ref().unwrap(), page_index)
                .unwrap();
            full.push_str(&page.text);
            if !page.next {
                break;
            }
            page_index += 1;
        }
        assert!(page_index > 0);
        assert_eq!(full, report);
    }
    #[test]
    fn r3_warning_history_survives_ui_log_rollover() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = crate::batch_warnings::Journal::new(dir.path());
        let mut log = String::new();
        report
            .record(
                std::path::Path::new("first.avif"),
                &["important first warning".into()],
            )
            .unwrap();
        log = super::append_log(&log, "important first warning");
        for i in 0..350 {
            log = super::append_log(&log, &format!("successful file {i}"));
        }
        assert!(log.lines().count() <= 200 && log.len() <= 65536);
        assert!(!log.contains("important first warning"));
        let summary = report.finish(351, 2, 3);
        assert_eq!(summary.files, 1);
        assert_eq!(summary.messages, 1);
        let page = crate::batch_warnings::read_page(summary.path.as_ref().unwrap(), 0).unwrap();
        assert!(page.text.contains("important first warning"));
        assert_eq!(
            crate::batch_warnings::clean_successes(356, 2, 3, summary.files),
            350
        ); // clean success count is separate from warned successes.
    }
}
