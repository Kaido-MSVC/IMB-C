//! 將選檔、資料夾匯入與 Windows 拖放轉成同一份待轉檔清單。
use crate::{AppWindow, append_log, engine, i18n, imports, preferences};
use slint::{
    ComponentHandle, SharedString, VecModel,
    language::StandardListViewItem,
    winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent},
};
use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

const INPUT_EXTENSIONS: &[&str] = &[
    "webp", "png", "jpg", "jpeg", "avif", "psd", "psb", "tif", "tiff", "jxl",
];

pub(crate) fn connect(
    app: &AppWindow,
    prefs: &Rc<RefCell<preferences::Preferences>>,
) -> Rc<RefCell<Vec<PathBuf>>> {
    let files = Rc::new(RefCell::new(Vec::new()));
    app.set_files(Rc::new(VecModel::default()).into());

    let weak = app.as_weak();
    let list = files.clone();
    app.on_file_selected(move |index| {
        let app = weak.unwrap();
        let files = list.borrow();
        let path = usize::try_from(index).ok().and_then(|i| files.get(i));
        app.set_selected_path(path.map(engine::display_path).unwrap_or_default().into());
    });

    let weak = app.as_weak();
    let list = files.clone();
    app.on_remove_file(move || {
        let app = weak.unwrap();
        let mut files = list.borrow_mut();
        if app.get_busy() {
            return;
        }
        if let Ok(index) = usize::try_from(app.get_current_file())
            && index < files.len()
        {
            files.remove(index);
            update_list(&app, &files);
        }
    });

    let weak = app.as_weak();
    let list = files.clone();
    app.on_clear_selection(move || {
        let app = weak.unwrap();
        if !app.get_busy() {
            list.borrow_mut().clear();
            update_list(&app, &[]);
        }
    });

    let weak = app.as_weak();
    let list = files.clone();
    let stored = prefs.clone();
    app.on_choose_files(move || {
        let app = weak.unwrap();
        let dialog = folder_dialog(&stored.borrow().source_folder)
            .add_filter(i18n::text(app.get_language_index(), 24), INPUT_EXTENSIONS);
        if let Some(paths) = app
            .window()
            .with_winit_window(|parent| dialog.set_parent(parent).pick_files())
            .flatten()
        {
            if let Some(parent) = paths.first().and_then(|p| p.parent()) {
                stored.borrow_mut().source_folder = parent.to_owned();
                app.invoke_preferences_changed();
            }
            import(&app, &list, paths);
        }
    });

    let weak = app.as_weak();
    let list = files.clone();
    let stored = prefs.clone();
    app.on_import_folder(move || {
        let app = weak.unwrap();
        let dialog = folder_dialog(&stored.borrow().source_folder);
        if let Some(folder) = app
            .window()
            .with_winit_window(|parent| dialog.set_parent(parent).pick_folder())
            .flatten()
        {
            stored.borrow_mut().source_folder = folder.clone();
            app.invoke_preferences_changed();
            import(&app, &list, vec![folder]);
        }
    });

    let weak = app.as_weak();
    let stored = prefs.clone();
    app.on_choose_folder(move || {
        let app = weak.unwrap();
        let dialog = folder_dialog(&stored.borrow().output_folder);
        if let Some(folder) = app
            .window()
            .with_winit_window(|parent| dialog.set_parent(parent).pick_folder())
            .flatten()
        {
            app.set_output_path(engine::display_path(&folder).into());
            stored.borrow_mut().output_folder = folder;
            app.set_ready(app.get_selected_count() > 0);
            app.invoke_preferences_changed();
        }
    });

    let weak = app.as_weak();
    let list = files.clone();
    let pending = Rc::new(RefCell::new(Vec::new()));
    app.window().on_winit_window_event(move |_, event| {
        if let Some(app) = weak.upgrade() {
            match event {
                WindowEvent::HoveredFile(_) => app.set_dragging(!app.get_busy()),
                WindowEvent::HoveredFileCancelled => app.set_dragging(false),
                WindowEvent::DroppedFile(path) => {
                    app.set_dragging(false);
                    if !app.get_busy()
                        && !crate::UI_CLOSING.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        let mut paths = pending.borrow_mut();
                        if paths.is_empty() {
                            let weak = weak.clone();
                            let list = list.clone();
                            let pending = pending.clone();
                            slint::Timer::single_shot(Duration::ZERO, move || {
                                let paths = std::mem::take(&mut *pending.borrow_mut());
                                if let Some(app) = weak.upgrade() {
                                    import(&app, &list, paths);
                                }
                            });
                        }
                        paths.push(path.clone());
                    }
                }
                _ => {}
            }
        }
        EventResult::Propagate
    });
    files
}

fn folder_dialog(path: &Path) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    if path.is_dir() {
        dialog.set_directory(path)
    } else {
        dialog
    }
}

fn import(app: &AppWindow, files: &RefCell<Vec<PathBuf>>, paths: Vec<PathBuf>) {
    if app.get_busy() || crate::UI_CLOSING.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let lang = app.get_language_index();
    let mut incoming = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() && is_regular_directory(&meta) => {
                match imports::folder(&path, app.get_recursive()) {
                    Ok((more, errors)) => {
                        incoming.extend(more);
                        warnings.extend(errors);
                    }
                    Err(error) => warnings.push(format!(
                        "{}：{} — {error}",
                        i18n::gui(lang, 38),
                        engine::display_path(&path)
                    )),
                }
            }
            Ok(meta) if meta.is_file() && engine::Format::input(&path).is_ok() => {
                incoming.push(path)
            }
            Ok(_) => warnings.push(format!(
                "{}：{}",
                i18n::gui(lang, 37),
                engine::display_path(&path)
            )),
            Err(error) => warnings.push(format!(
                "{}：{} — {error}",
                i18n::gui(lang, 38),
                engine::display_path(&path)
            )),
        }
    }
    let mut files = files.borrow_mut();
    let added = append_unique(&mut files, incoming);
    if added > 0 {
        update_list(app, &files);
        app.set_show_log(false);
    } else {
        app.set_show_log(true);
    }
    let mut log = append_log(
        &app.get_log(),
        &if added == 0 {
            i18n::gui(lang, 46).to_owned()
        } else {
            format!("{}：{added}", i18n::gui(lang, 35))
        },
    );
    for warning in warnings {
        log = append_log(&log, &warning);
    }
    app.set_log(log.into());
}

fn is_regular_directory(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // 匯入根目錄和子目錄都不穿越重新解析點。
    meta.file_attributes() & 0x400 == 0
}

fn append_unique(files: &mut Vec<PathBuf>, incoming: Vec<PathBuf>) -> usize {
    let before = files.len();
    let mut seen: HashSet<_> = files.iter().cloned().collect();
    files.extend(
        incoming
            .into_iter()
            .filter(|path| seen.insert(path.clone())),
    );
    files.len() - before
}

fn update_list(app: &AppWindow, files: &[PathBuf]) {
    let model = files
        .iter()
        .map(|path| {
            let text = path.file_name().map_or_else(
                || engine::display_path(path).into(),
                |name| SharedString::from(name.to_string_lossy().as_ref()),
            );
            StandardListViewItem::from(text)
        })
        .collect::<Vec<_>>();
    app.set_current_file(-1);
    app.set_selected_path("".into());
    app.set_files(Rc::new(VecModel::from(model)).into());
    app.set_selected_count(files.len().min(i32::MAX as usize) as i32);
    app.set_ready(!files.is_empty() && !app.get_output_path().is_empty());
}

#[cfg(test)]
mod tests {
    use super::append_unique;

    #[test]
    fn adding_files_keeps_order_and_skips_duplicates() {
        let mut files = vec!["a.png".into(), "b.jpg".into()];
        assert_eq!(
            append_unique(
                &mut files,
                vec!["a.png".into(), "c.png".into(), "c.png".into()]
            ),
            1,
        );
        assert_eq!(
            files,
            ["a.png", "b.jpg", "c.png"].map(std::path::PathBuf::from)
        );
    }
}
