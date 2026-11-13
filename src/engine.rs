#[path = "color_diagnostics.rs"]
mod color_diagnostics;
#[path = "controlled_temp.rs"]
mod controlled_temp;
#[path = "engine_limits.rs"]
mod engine_limits;
#[path = "engine_memory.rs"]
mod engine_memory;
#[path = "instance_ledger.rs"]
mod instance_ledger;
#[path = "low_integrity.rs"]
mod low_integrity;
#[cfg(windows)]
#[path = "process_guard.rs"]
mod process_guard;
#[cfg(test)]
thread_local! { static TEST_NATIVE_PEAK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! {
    static TEST_READER_PANIC: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static TEST_TREE_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_TREE_CONFIRMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[allow(dead_code)] // GUI localization; standalone verification binaries share this engine.
pub fn localize_color_error(lang: i32, detail: &str) -> String {
    color_diagnostics::localize(lang, detail)
}
use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    plain_error_text(e.to_string())
}

/// The one way a path becomes user-facing text (GUI log, warning report, dialogs, errors).
/// Drops the Win32 verbatim prefix (`\\?\D:\…` → `D:\…`, `\\?\UNC\server\share` →
/// `\\server\share`) without quotes or escaping. File operations keep the original path.
pub fn display_path(path: impl AsRef<Path>) -> String {
    plain_path_text(&path.as_ref().to_string_lossy())
}

fn plain_path_text(text: &str) -> String {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        // Only drive paths; \\?\Volume{…} and \\?\GLOBALROOT have no shorter equivalent.
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return rest.to_owned();
        }
    }
    text.to_owned()
}

/// tempfile 3.27 reports `<os error> at path "<Debug path>"`, which doubles every
/// backslash and adds quotes. Rewrite that suffix as a plain, copyable Windows path.
pub fn plain_error_text(message: String) -> String {
    let Some((cause, quoted)) = message.rsplit_once(" at path ") else {
        return message;
    };
    let Some(inner) = quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
        return message;
    };
    let mut path = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some(next @ ('\\' | '"' | '\''))) => {
                path.push(next);
                chars.next();
            }
            _ => path.push(c),
        }
    }
    format!("{cause}；位置 {}", plain_path_text(&path))
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Webp,
    Png,
    Jpeg,
    Avif,
    Psd,
    Tiff,
    Jxl,
}
impl Format {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Png,
            2 => Self::Jpeg,
            3 => Self::Avif,
            4 => Self::Psd,
            5 => Self::Tiff,
            6 => Self::Jxl,
            _ => Self::Webp,
        }
    }
    fn coder(self) -> &'static str {
        match self {
            Self::Webp => "webp:-",
            Self::Png => "png:-",
            Self::Jpeg => "jpeg:-",
            Self::Avif => "avif:-",
            Self::Psd => "psd:-",
            Self::Tiff => "tiff:-",
            Self::Jxl => "jxl:-",
        }
    }
    fn input_coder(self) -> &'static str {
        // Read the saved PSD composite, never interpret layer frames as animation.
        if matches!(self, Self::Psd) {
            "psd:-[0]"
        } else {
            self.coder()
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Avif => "avif",
            Self::Psd => "psd",
            Self::Tiff => "tiff",
            Self::Jxl => "jxl",
        }
    }
    pub fn input(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "webp" => Ok(Self::Webp),
            "png" => Ok(Self::Png),
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "avif" => Ok(Self::Avif),
            "psd" => Ok(Self::Psd),
            "tif" | "tiff" => Ok(Self::Tiff),
            "jxl" => Ok(Self::Jxl),
            _ => Err("僅支援 WebP、PNG、JPEG、AVIF、PSD、TIFF、JXL；不支援 HEIC/HEIF".into()),
        }
    }
}

pub fn locate_engine() -> PathBuf {
    // Development and release both use this copy's private engine.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("ImageMagick").join("magick.exe")))
        .unwrap_or_default()
}

/// 啟動時回收已終止 IMB-C 執行個體的殘留（P1-C）：只處理其擁有權日誌中 File ID 相符、
/// 未被使用的項目，並終止其建立→納管空窗留下的暫停子程序。沒有需要回報的事項時回傳空集合。
#[allow(dead_code)] // GUI 使用；驗收工具共用 engine.rs
pub fn recover_stale_temp() -> Vec<String> {
    let report = match instance_ledger::global().and_then(|l| l.recover()) {
        Ok(report) => report,
        Err(e) => {
            return vec![format!(
                "殘留暫存回收未執行：{}",
                plain_error_text(e.to_string())
            )];
        }
    };
    let mut lines = Vec::new();
    if report.removed > 0 || report.orphans > 0 {
        lines.push(format!(
            "已回收先前異常結束留下的暫存 {} 項、暫停子程序 {} 個",
            report.removed, report.orphans
        ));
    }
    for kept in report.kept {
        lines.push(format!(
            "先前的暫存仍在使用或無法刪除，已保留並於下次啟動重試：{}",
            plain_error_text(kept)
        ));
    }
    lines
}
#[allow(dead_code)] // GUI 使用；驗收工具共用 engine.rs
pub fn shutdown_instance() {
    instance_ledger::shutdown();
}

// One context per worker thread; scoped to a single conversion, including verification.
thread_local! {
    static JOB: std::cell::RefCell<Option<(PathBuf, Instant, bool)>> = const { std::cell::RefCell::new(None) };
    static JOB_BUDGET: std::cell::RefCell<Option<std::sync::Arc<engine_limits::Budget>>> = const { std::cell::RefCell::new(None) };
    static JOB_IMAGE: std::cell::Cell<(u64, u64, u64)> = const { std::cell::Cell::new((0, 16, 4)) };
    static JOB_STORAGE: std::cell::RefCell<Option<std::sync::Arc<controlled_temp::ControlledDir>>> = const { std::cell::RefCell::new(None) };
    static JOB_CWD: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    // 跨執行個體磁碟預留：工作結束（Job drop）時釋放。
    static JOB_RESERVATIONS: std::cell::RefCell<Vec<instance_ledger::Reservation>> = const { std::cell::RefCell::new(Vec::new()) };
    static WARNINGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
struct Job(
    Option<std::sync::Arc<controlled_temp::ControlledDir>>,
    Option<std::sync::Arc<controlled_temp::ControlledDir>>,
);
impl Job {
    fn new() -> Result<Self> {
        Self::new_in(&controlled_temp::preferred_root().map_err(err)?)
    }
    fn new_in(requested: &Path) -> Result<Self> {
        WARNINGS.with(|warnings| warnings.borrow_mut().clear());
        JOB_IMAGE.with(|v| v.set((0, 16, 4)));
        // Low IL 引擎無法存取 UNC／網路磁碟機；暫存改放本機 NTFS 並明確告知實際路徑。
        // 圖片輸入與輸出仍可在 UNC，由 Rust 控制代碼讀寫。
        let (root, redirected) = controlled_temp::local_engine_root(requested).map_err(err)?;
        let root = root.as_path();
        if let Some(original) = redirected {
            WARNINGS.with(|w| {
                w.borrow_mut().push(format!(
                    "暫存根目錄 {} 是網路路徑，Low IL 引擎無法存取；本次改用本機受控暫存 {}",
                    display_path(&original),
                    display_path(root)
                ))
            });
        }
        let dir = std::sync::Arc::new(
            controlled_temp::ControlledDir::create(root, "imb-c-work-").map_err(err)?,
        );
        // 為原生快取的兩層子目錄與隨機檔名保留 MAX_PATH 餘裕。
        // 來源／中繼的控制代碼 I/O 仍可使用原本的長路徑或 UNC。
        #[cfg(windows)]
        let cwd = if dir.path().to_string_lossy().encode_utf16().count() > 120 {
            let short = std::sync::Arc::new(
                controlled_temp::ControlledDir::create(
                    &controlled_temp::local_engine_root(
                        &controlled_temp::preferred_root().map_err(err)?,
                    )
                    .map_err(err)?
                    .0,
                    "imb-c-cwd-",
                )
                .map_err(err)?,
            );
            if short.path().to_string_lossy().encode_utf16().count() > 120 {
                return Err(
                    "無可用的短引擎工作路徑；請設定較短的 MAGICK_TEMPORARY_PATH 路徑".into(),
                );
            }
            Some(short)
        } else {
            None
        };
        #[cfg(not(windows))]
        let cwd: Option<tempfile::TempDir> = None;
        JOB_CWD.with(|j| *j.borrow_mut() = Some(cwd.as_ref().unwrap_or(&dir).path().to_owned()));
        JOB_STORAGE.with(|j| *j.borrow_mut() = Some(cwd.as_ref().unwrap_or(&dir).clone()));
        JOB.with(|j| {
            *j.borrow_mut() = Some((
                dir.path().to_owned(),
                Instant::now() + Duration::from_secs(600),
                false,
            ))
        });
        JOB_BUDGET
            .with(|b| *b.borrow_mut() = Some(std::sync::Arc::new(engine_limits::Budget::new())));
        Ok(Self(Some(dir), cwd))
    }
    fn finish<T>(mut self, mut result: Result<T>) -> Result<T> {
        let retained = JOB.with(|j| j.borrow().as_ref().is_some_and(|v| v.2));
        JOB_STORAGE.with(|j| *j.borrow_mut() = None);
        for dir in [self.0.take(), self.1.take()].into_iter().flatten() {
            let dir = match std::sync::Arc::try_unwrap(dir) {
                Ok(dir) => dir,
                Err(dir) => {
                    // 保留狀態屬於目錄本身，最後一個共享參照釋放時仍不得清理。
                    dir.retain();
                    let warning = format!("暫存目錄仍被使用，已保留 {}", display_path(dir.path()));
                    result = if retained {
                        Err(format!("{}；{warning}", result.err().unwrap_or_default()))
                    } else {
                        cleanup_result(result, Err(warning))
                    };
                    continue;
                }
            };
            if retained {
                let path = dir.keep();
                result = Err(format!(
                    "{}；工作目錄保留於 {}",
                    result.err().unwrap_or_default(),
                    display_path(&path)
                ));
            } else {
                let path = dir.path().to_owned();
                result = cleanup_result(result, dir.close().map_err(|e| format!("工作暫存清理失敗 {}：{e}；可能累積磁碟用量，請在所有工作停止後檢查此目錄", display_path(&path))));
            }
        }
        result
    }
    fn finish_with_warnings<T>(self, result: Result<T>) -> Result<(T, Vec<String>)> {
        let result = self.finish(result);
        let warnings = WARNINGS.with(|w| std::mem::take(&mut *w.borrow_mut()));
        match result {
            Ok(value) => Ok((value, warnings)),
            Err(e) => Err(if warnings.is_empty() {
                e
            } else {
                format!("{e}；{}", warnings.join("；"))
            }),
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        let retained =
            JOB.with(|j| j.borrow_mut().take().is_some_and(|v| v.2)) || std::thread::panicking();
        if retained {
            for dir in [&self.0, &self.1].into_iter().flatten() {
                dir.retain();
            }
        }
        JOB_STORAGE.with(|j| *j.borrow_mut() = None);
        JOB_CWD.with(|j| *j.borrow_mut() = None);
        JOB_BUDGET.with(|b| *b.borrow_mut() = None);
        JOB_RESERVATIONS.with(|r| r.borrow_mut().clear());
    }
}
fn cleanup_result<T>(result: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (result, cleanup) {
        (Ok(value), Err(warning)) => {
            WARNINGS.with(|w| w.borrow_mut().push(warning));
            Ok(value)
        }
        (Err(e), Err(warning)) => Err(format!("{e}；{warning}")),
        (result, Ok(())) => result,
    }
}
fn job_dir() -> PathBuf {
    JOB.with(|j| {
        j.borrow()
            .as_ref()
            .map(|v| v.0.clone())
            .unwrap_or_else(std::env::temp_dir)
    })
}
fn job_cwd() -> PathBuf {
    JOB_CWD.with(|j| j.borrow().clone().unwrap_or_else(job_dir))
}
fn job_file() -> Result<File> {
    tempfile::tempfile_in(job_dir()).map_err(err)
}
fn check_job(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    JOB.with(|j| {
        if j.borrow().as_ref().is_some_and(|v| Instant::now() >= v.1) {
            Err("單檔工作超過 600 秒總期限，未保存輸出".into())
        } else {
            Ok(())
        }
    })
}
fn validate_engine(engine: &Path) -> Result<()> {
    if !engine.is_absolute() || !engine.is_file() {
        return Err("無法定位絕對路徑的私有 ImageMagick 引擎，未啟動子程序".into());
    }
    let home = engine.parent().ok_or("引擎路徑無父目錄")?;
    let policy =
        std::fs::read(home.join("policy.xml")).map_err(|e| format!("私有 policy 缺漏：{e}"))?;
    if policy != include_bytes!("../assets/policy-v13.xml") {
        return Err("私有 policy 與 v1.3 預期內容不符，請校驗 runtime".into());
    }
    for module in ["miff", "meta", "rgb"] {
        if !home
            .join(format!("modules/coders/IM_MOD_RL_{module}_.dll"))
            .is_file()
        {
            return Err(format!("私有 runtime 缺少 {module} coder，請校驗 runtime"));
        }
    }
    Ok(())
}
// 核心已將組態搜尋固定在隨包 DLL 目錄；仍在讀圖前核對清單與禁用 coder，
// 避免舊版／錯誤 runtime 或私有 policy 異動被默默接受。
fn validate_policy_listing(text: &str, private: &Path) -> Result<()> {
    let private = private.canonicalize().map_err(err)?;
    let mut in_private = false;
    let mut saw_private = false;
    let mut saw_private_rule = false;
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(path) = line.strip_prefix("Path: ") {
            in_private = path != "[built-in]"
                && Path::new(path)
                    .canonicalize()
                    .map_err(|e| format!("無法核對生效 policy 路徑 {path}：{e}"))?
                    == private;
            saw_private |= in_private;
        } else if let Some(domain) = line.strip_prefix("Policy: ") {
            if in_private {
                saw_private_rule = true;
            } else if domain != "Undefined" {
                return Err(format!(
                    "偵測到私有 runtime 以外的生效 policy（{domain}），可能放寬或干擾限制，未讀取圖片；請檢查系統 ImageMagick policy"
                ));
            }
        }
    }
    if !saw_private || !saw_private_rule {
        return Err("無法確認私有 policy 已載入，未讀取圖片".into());
    }
    Ok(())
}
fn verify_active_policy(engine: &Path, cancel: &AtomicBool) -> Result<()> {
    let source = job_file()?;
    let mut out = job_file()?;
    run(engine, &["-list", "policy"], &source, &out, cancel)?;
    if out.metadata().map_err(err)?.len() > 1024 * 1024 {
        return Err("policy 清單過大，無法安全核對".into());
    }
    out.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut listing = String::new();
    out.read_to_string(&mut listing).map_err(err)?;
    validate_policy_listing(
        &listing,
        &engine.parent().ok_or("引擎父目錄缺漏")?.join("policy.xml"),
    )?;
    // Exercise a normally available raw writer that our coder policy forbids.
    // This also detects common broad stealth grants omitted from -list policy.
    let mut source = job_file()?;
    use std::io::Write;
    source
        .write_all(include_bytes!("../tests/fixtures/seed.png"))
        .map_err(err)?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let output = job_file()?;
    match run(engine, &["png:-", "rgb:-"], &source, &output, cancel) {
        Err(e)
            if (e.contains("security policy") || e.contains("not authorized"))
                && output.metadata().map_err(err)?.len() == 0 =>
        {
            Ok(())
        }
        Err(e) if e == "已取消" => Err(e),
        result => Err(format!(
            "私有 policy 禁用 RGB coder 檢查未通過，未讀取圖片：{result:?}"
        )),
    }
}

pub fn locate_jxl_tools(executable: std::io::Result<PathBuf>) -> Result<PathBuf> {
    let executable = executable.map_err(|e| format!("無法定位 JPEG ↔ JXL 工具：{e}"))?;
    if !executable.is_absolute() {
        return Err("執行檔路徑必須為絕對路徑".into());
    }
    let tools = executable.with_file_name("JXL");
    if !tools.join("cjxl.exe").is_file() || !tools.join("djxl.exe").is_file() {
        return Err("缺少私有 JPEG ↔ JXL 工具 cjxl.exe / djxl.exe".into());
    }
    Ok(tools)
}
// ImageMagick 7.1.2-34: primaries/transfer/matrix/full_range (coders/heic.c).
fn validate_avif_cicp(text: &str) -> Result<()> {
    let fields = text
        .trim()
        .split('/')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>();
    match fields.as_deref() {
        Ok([1, 1 | 13, 0 | 1 | 6, 0 | 1]) => Ok(()),
        _ => Err(format!(
            "此 AVIF 沒有 ICC，CICP 無法確認為支援的 SDR（{}）；HDR／廣色域或未知資訊未保存輸出，請使用已嵌入正確 ICC 的來源",
            text.trim()
        )),
    }
}

fn private_engine_path(home: &Path) -> Result<OsString> {
    let system = windows_directory()?.join("System32");
    std::env::join_paths([home, system.as_path()]).map_err(err)
}

fn windows_directory() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    let mut buffer = [0u16; 32768];
    let count = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if count == 0 {
        return Err(format!(
            "無法定位 Windows 系統目錄：{}",
            std::io::Error::last_os_error()
        ));
    }
    if count >= buffer.len() {
        return Err("Windows 系統目錄長度無效".into());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..count])))
}

fn child_environment(
    command: &mut Command,
    home: &Path,
    scratch: &Path,
    profile: &Path,
) -> Result<()> {
    let windows = windows_directory()?;
    command
        .env_clear()
        .env("SystemRoot", &windows)
        .env("WINDIR", &windows)
        .env("COMSPEC", windows.join("System32/cmd.exe"))
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("PATH", private_engine_path(home)?)
        .env("MAGICK_TEMPORARY_PATH", scratch)
        .env("TEMP", scratch)
        .env("TMP", scratch)
        .env("TMPDIR", scratch)
        .env("HOME", profile)
        .env("USERPROFILE", profile)
        .env("APPDATA", profile)
        .env("LOCALAPPDATA", profile)
        .env("XDG_CONFIG_HOME", profile)
        .env("MAGICK_THREAD_LIMIT", "1")
        .env("OMP_NUM_THREADS", "1")
        .env("MAGICK_HOME", home)
        .env("MAGICK_CONFIGURE_PATH", home)
        .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
        .env("MAGICK_FILTER_MODULE_PATH", home.join("modules/filters"));
    Ok(())
}

fn run(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
) -> Result<()> {
    run_with_timeout(
        engine,
        args,
        source,
        output,
        cancel,
        Duration::from_secs(600),
    )
}

struct Invocation {
    scratch: Option<controlled_temp::ControlledDir>,
    stopped: bool,
}
impl Drop for Invocation {
    fn drop(&mut self) {
        if !self.stopped {
            JOB.with(|j| {
                if let Some(job) = j.borrow_mut().as_mut() {
                    job.2 = true;
                }
            });
            if let Some(dir) = self.scratch.take() {
                let _ = dir.keep();
            }
        }
    }
}
fn run_with_timeout(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<()> {
    check_job(cancel)?;
    // Only this freshly created random directory is owned by this invocation.
    // Never scan/delete global magick-* files or a directory from a previous run.
    let scratch = JOB_STORAGE
        .with(|j| match j.borrow().as_ref() {
            Some(parent) => parent.child("imb-c-job-"),
            None => controlled_temp::ControlledDir::create(
                &controlled_temp::local_engine_root(&controlled_temp::preferred_root()?)?.0,
                "imb-c-job-",
            ),
        })
        .map_err(err)?;
    let scratch_path = scratch.path().to_path_buf();
    // 個人資料區可能建立子目錄，與只接受一般檔案的原生快取監測分開。
    let sandbox =
        low_integrity::Sandbox::new().map_err(|e| format!("無法建立 Low IL 隔離：{e}"))?;
    scratch
        .allow_sandbox(&sandbox)
        .map_err(|e| format!("無法設定 Low IL 工作目錄：{e}"))?;
    let native = scratch.child("imb-c-cache-").map_err(err)?;
    native
        .allow_sandbox(&sandbox)
        .map_err(|e| format!("無法設定 Low IL 快取目錄：{e}"))?;
    controlled_temp::check_free(&scratch_path, 2 * 1024 * 1024 * 1024).map_err(err)?;
    let mut invocation = Invocation {
        scratch: Some(scratch),
        stopped: true,
    };
    let result = run_child(
        engine,
        args,
        source,
        output,
        cancel,
        timeout,
        native.path(),
        &scratch_path,
        &mut invocation.stopped,
        &sandbox,
    );
    let scratch = invocation.scratch.take().unwrap();
    if !invocation.stopped {
        // A failed wait cannot establish that the child has released its files.
        // Preserve this directory and report it instead of racing a live child.
        JOB.with(|j| {
            if let Some(job) = j.borrow_mut().as_mut() {
                job.2 = true;
            }
        });
        let _ = native.keep();
        let retained = scratch.keep();
        return Err(format!(
            "{}；無法確認子程序停止，暫存保留於 {}",
            result.unwrap_err(),
            display_path(&retained)
        ));
    }
    let result = cleanup_result(
        result,
        native.close().map_err(|e| format!("原生暫存清理失敗：{e}")),
    );
    cleanup_result(
        result,
        scratch
            .close()
            .map_err(|e| format!("引擎暫存清理失敗 {}：{e}", display_path(&scratch_path))),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_child(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
    timeout: Duration,
    scratch: &Path,
    profile: &Path,
    stopped: &mut bool,
    sandbox: &low_integrity::Sandbox,
) -> Result<()> {
    let stderr = job_file()?;
    if engine
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("magick.exe"))
    {
        validate_engine(engine)?;
    }
    let mut command = Command::new(engine);
    let home = engine
        .parent()
        .filter(|p| p.is_absolute())
        .ok_or("引擎需要絕對父目錄")?;
    child_environment(&mut command, home, scratch, profile)?;
    command
        .args(args)
        .env("MAGICK_TEMPORARY_PATH", scratch)
        .env("TEMP", scratch)
        .env("TMP", scratch)
        .env("MAGICK_THREAD_LIMIT", "1")
        .env("OMP_NUM_THREADS", "1")
        .stdin(Stdio::from(source.try_clone().map_err(err)?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = engine.parent().filter(|p| p.is_absolute()) {
        // Child-only configuration: never change the user's PATH or installed ImageMagick.
        command
            .current_dir(
                JOB_CWD
                    .with(|j| j.borrow().clone())
                    .unwrap_or_else(|| home.to_owned())
                    .canonicalize()
                    .map_err(|e| format!("引擎工作目錄解析失敗：{e}"))?,
            )
            .env("PATH", private_engine_path(home)?)
            .env("MAGICK_HOME", home)
            .env("MAGICK_CONFIGURE_PATH", home)
            .env("XDG_CONFIG_HOME", profile)
            .env("HOME", profile)
            .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
            .env("MAGICK_FILTER_MODULE_PATH", home.join("modules/filters"))
            .env_remove("MAGICK_CONFIGURE_BUILD_PATH");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let (pixels, depth, channels) = JOB_IMAGE.with(|v| v.get());
    let memory = engine_memory::Lease::acquire(
        engine_memory::profile(pixels, depth, channels, args),
        || check_job(cancel),
    )?;
    #[cfg(windows)]
    let guard = process_guard::ProcessGuard::with_memory_limit(memory.0)
        .map_err(|e| format!("無法建立子程序資源防護：{e}"))?;
    // 啟動錯誤先確認整個 Job；只有停止狀態未知才保留 Low 暫存。
    *stopped = false;
    #[cfg(windows)]
    let mut child = match sandbox.spawn(&command, source, guard.handle()) {
        Ok(child) => child,
        Err(error) => {
            let tree = guard.stop_and_wait();
            *stopped = tree.is_ok();
            let primary = format!("無法安全啟動 {}：{error}", display_path(engine));
            return Err(match tree {
                Ok(()) => primary,
                Err(e) => format!("{primary}；無法確認啟動失敗程序樹停止：{e}"),
            });
        }
    };
    #[cfg(not(windows))]
    let mut child = command.spawn().map_err(err)?;
    *stopped = false;
    let stdout = child.stdout.take().ok_or("子程序 stdout 缺漏")?;
    let error_pipe = child.stderr.take().ok_or("子程序 stderr 缺漏")?;
    let destination = output.try_clone().map_err(err)?;
    let error_destination = stderr.try_clone().map_err(err)?;
    let budget = JOB_BUDGET
        .with(|b| b.borrow().clone())
        .unwrap_or_else(|| std::sync::Arc::new(engine_limits::Budget::new()));
    let failed = AtomicBool::new(false);
    let piped = std::sync::atomic::AtomicU64::new(0);
    let output_limit = engine_limits::output_limit();
    let native_write_limit = engine_limits::native_write_limit();
    let readers_stopped = AtomicBool::new(false);
    #[cfg(test)]
    let panic_readers = TEST_READER_PANIC.with(|v| v.get());
    let result = std::thread::scope(|scope| {
        let _stop_on_unwind = engine_limits::StopReaders(&readers_stopped);
        let out_reader = scope.spawn(|| {
            #[cfg(test)]
            engine_limits::TEST_PANIC_NEXT.with(|v| v.set(panic_readers & 1 != 0));
            engine_limits::pump(
                engine_limits::PipeReader::new(stdout, &readers_stopped),
                destination,
                output_limit,
                &budget,
                &failed,
                &piped,
            )
        });
        let err_reader = scope.spawn(|| {
            #[cfg(test)]
            engine_limits::TEST_PANIC_NEXT.with(|v| v.set(panic_readers & 2 != 0));
            engine_limits::pump(
                engine_limits::PipeReader::new(error_pipe, &readers_stopped),
                error_destination,
                engine_limits::STDERR_LIMIT,
                &budget,
                &failed,
                &piped,
            )
        });
        let result = (|| {
            let started = Instant::now();
            let mut native_peak = 0u64;
            let mut disk_check = Instant::now();
            loop {
                let resource_error = (|| {
                    let size = controlled_temp::native_size(scratch).map_err(err)?;
                    if size > 8 * 1024 * 1024 * 1024 {
                        return Err("原生暫存觀察用量超過 8 GiB，未保存輸出".to_owned());
                    }
                    if size > native_peak {
                        budget.reserve(size - native_peak).map_err(err)?;
                        native_peak = size;
                        #[cfg(test)]
                        TEST_NATIVE_PEAK.with(|v| v.set(v.get().max(size)));
                    }
                    // 核心 Job I/O 計數：涵蓋受控目錄外、子目錄、刪除後仍開啟的寫入。
                    // 管線內尚未讀出的少量資料會使估計略偏高（保守方向）。
                    #[cfg(windows)]
                    {
                        let native_written = guard
                            .write_bytes()
                            .map_err(|e| format!("無法讀取子程序寫入計數：{e}"))?
                            .saturating_sub(piped.load(Ordering::Relaxed));
                        if native_written > native_write_limit {
                            return Err(format!(
                                "引擎程序樹累計寫入（不含標準串流）超過 {} GiB，未保存輸出",
                                native_write_limit / (1024 * 1024 * 1024)
                            ));
                        }
                    }
                    if disk_check.elapsed() >= Duration::from_millis(250) {
                        controlled_temp::check_free(scratch, 2 * 1024 * 1024 * 1024)
                            .map_err(err)?;
                        disk_check = Instant::now();
                    }
                    Ok::<(), String>(())
                })()
                .err();
                if resource_error.is_some()
                    || failed.load(Ordering::Relaxed)
                    || check_job(cancel).is_err()
                    || started.elapsed() > timeout
                {
                    #[cfg(windows)]
                    guard.terminate();
                    let _ = child.kill();
                    child
                        .wait()
                        .map_err(|e| format!("等待子程序停止失敗：{e}"))?;
                    *stopped = true;
                    break Err(if let Some(reason) = resource_error {
                        reason
                    } else if failed.load(Ordering::Relaxed) {
                        "引擎串流超限或寫入失敗，未保存輸出".into()
                    } else if let Err(reason) = check_job(cancel) {
                        reason
                    } else {
                        format!("超過單次執行 {} 秒限制", timeout.as_secs())
                    });
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        *stopped = true;
                        if status.success() {
                            break Ok(());
                        }
                        break Err(format!(
                            "{} 失敗 ({status})",
                            engine.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                    Err(e) => {
                        #[cfg(windows)]
                        guard.terminate();
                        let _ = child.kill();
                        child
                            .wait()
                            .map_err(|wait| format!("{e}；等待子程序停止失敗：{wait}"))?;
                        *stopped = true;
                        break Err(err(e));
                    }
                }
            }
        })();
        // 正常退出也關閉 Job，避免後代持有串流而阻止讀取執行緒結束。
        #[cfg(windows)]
        let tree_result = guard.stop_and_wait();
        readers_stopped.store(true, Ordering::Release);
        #[cfg(test)]
        let tree_result = {
            TEST_TREE_CONFIRMED.with(|v| v.set(tree_result.is_ok()));
            if tree_result.is_ok() && TEST_TREE_FAILURE.with(|v| v.get()) {
                Err(std::io::Error::other("injected tree confirmation failure"))
            } else {
                tree_result
            }
        };
        #[cfg(test)]
        if std::env::var_os("IMBC_TEST_RESOURCE_LOG").is_some() {
            eprintln!(
                "RESOURCE peak_job_commit={} job_write_bytes={} piped={} scratch={} args={:?}",
                guard.peak_committed().unwrap_or(0),
                guard.write_bytes().unwrap_or(0),
                piped.load(Ordering::Relaxed),
                scratch.display(),
                args
            );
        }
        #[cfg(windows)]
        drop(guard);
        finish_child_io(
            stopped,
            result,
            tree_result,
            || out_reader.join(),
            || err_reader.join(),
        )
    });
    if let Err(e) = result {
        // GUI 以此精確值區分取消；引擎的告知文字不能改變取消分類。
        if e == "已取消" {
            return Err(e);
        }
        let mut stderr = stderr;
        stderr
            .seek(SeekFrom::Start(0))
            .map_err(|secondary| format!("{e}；stderr 診斷定位失敗：{secondary}"))?;
        let mut detail = Vec::new();
        stderr
            .take(4096)
            .read_to_end(&mut detail)
            .map_err(|secondary| format!("{e}；stderr 診斷讀取失敗：{secondary}"))?;
        let detail = String::from_utf8_lossy(&detail);
        return Err(if detail.trim().is_empty() {
            e
        } else {
            format!("{e}：{}", detail.trim())
        });
    }
    Ok(())
}

// 先記錄程序樹確認結果，再收回兩個執行緒；任何一個 panic 都不能略過另一個 join。
fn finish_child_io(
    stopped: &mut bool,
    result: Result<()>,
    tree: std::io::Result<()>,
    out: impl FnOnce() -> std::thread::Result<std::io::Result<()>>,
    error: impl FnOnce() -> std::thread::Result<std::io::Result<()>>,
) -> Result<()> {
    *stopped = tree.is_ok();
    let out = out();
    let error = error();
    let mut errors = Vec::new();
    if let Err(e) = tree {
        errors.push(format!("無法確認子程序樹停止：{e}"));
    }
    if let Err(e) = result {
        errors.push(e);
    }
    for (label, joined) in [("stdout", out), ("stderr", error)] {
        match joined {
            Err(_) => errors.push(format!("{label} 讀取執行緒異常")),
            Ok(Err(e)) => errors.push(format!("{label} 寫入失敗，未保存輸出：{e}")),
            Ok(Ok(())) => (),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

// IM 7.1.2-34 rejects every zero-length image resource. CSP writes valid
// empty alpha-name resources (1006 / 1045). Normalize only those empty records
// in a private stream; all other resource, layer and composite bytes are copied.
fn psd_compatible_source(source: &mut File, cancel: &AtomicBool) -> Result<Option<File>> {
    fn long(file: &mut File) -> Result<u32> {
        let mut bytes = [0; 4];
        file.read_exact(&mut bytes)
            .map_err(|e| format!("PSD 資源區塊截斷：{e}"))?;
        Ok(u32::from_be_bytes(bytes))
    }
    let length = source.metadata().map_err(err)?.len();
    source.seek(SeekFrom::Start(26)).map_err(err)?;
    let color_length = u64::from(long(source)?);
    let resource_length_pos = 30 + color_length;
    if resource_length_pos + 4 > length {
        return Err("PSD 色彩資料區塊超出檔案範圍".into());
    }
    source
        .seek(SeekFrom::Start(resource_length_pos))
        .map_err(err)?;
    let resource_length = u64::from(long(source)?);
    let start = resource_length_pos + 4;
    let end = start + resource_length;
    if end > length {
        return Err("PSD 資源區塊超出檔案範圍".into());
    }
    let mut at = start;
    let mut removed = Vec::new();
    while at < end {
        check_job(cancel)?;
        if end - at < 12 {
            break; // Preserve unparsed trailing bytes for the engine.
        }
        source.seek(SeekFrom::Start(at)).map_err(err)?;
        let mut header = [0; 7];
        source.read_exact(&mut header).map_err(err)?;
        if &header[..4] != b"8BIM" {
            break; // The engine stops resource parsing here; do not reject its input.
        }
        let id = u16::from_be_bytes(header[4..6].try_into().unwrap());
        let name_size = (u64::from(header[6]) + 2) & !1;
        let size_pos = at + 6 + name_size;
        if size_pos + 4 > end {
            break; // No complete length field; never read outside the resource area.
        }
        source.seek(SeekFrom::Start(size_pos)).map_err(err)?;
        let size = u64::from(long(source)?);
        let data_end = size_pos
            .checked_add(4)
            .and_then(|v| v.checked_add(size))
            .ok_or("PSD 資源長度溢位")?;
        if data_end > end {
            return Err("PSD 資源資料超出區塊範圍".into());
        }
        // A final odd payload may omit its padding; preserve it unchanged.
        let next = data_end
            .checked_add(size & 1)
            .ok_or("PSD 資源長度溢位")?
            .min(end);
        if size == 0 && matches!(id, 1006 | 1045) {
            if removed.len() >= 65536 {
                return Err("PSD 空白名稱資源數量超限".into());
            }
            removed.push((at, next));
        }
        at = next;
    }
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    if removed.is_empty() {
        return Ok(None);
    }
    let removed_size: u64 = removed.iter().map(|(a, b)| b - a).sum();
    let mut out = job_file()?;
    copy_bytes(source, &mut out, resource_length_pos, cancel)?;
    write_job_bytes(
        &mut out,
        &((resource_length - removed_size) as u32).to_be_bytes(),
    )?;
    source.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut cursor = start;
    for (a, b) in removed {
        copy_bytes(source, &mut out, a - cursor, cancel)?;
        source.seek(SeekFrom::Start(b)).map_err(err)?;
        cursor = b;
    }
    copy_bytes(source, &mut out, length - cursor, cancel)?;
    out.seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(Some(out))
}
fn copy_bytes(
    source: &mut File,
    target: &mut File,
    mut count: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut buffer = [0; 65536];
    while count > 0 {
        check_job(cancel)?;
        let n = count.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..n]).map_err(err)?;
        write_job_bytes(target, &buffer[..n])?;
        count -= n as u64;
    }
    Ok(())
}

fn write_job_bytes(target: &mut File, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let end = target
        .stream_position()
        .map_err(err)?
        .checked_add(bytes.len() as u64)
        .ok_or("暫存長度溢位")?;
    if end > engine_limits::OUTPUT_LIMIT {
        return Err("暫存寫入超過 16 GiB 上限".into());
    }
    JOB_BUDGET.with(|b| {
        if let Some(budget) = b.borrow().as_ref() {
            budget.reserve(bytes.len() as u64).map_err(err)?;
        }
        Ok::<(), String>(())
    })?;
    target.write_all(bytes).map_err(err)
}

// PNG's animation control chunk precedes image data. The ordinary PNG coder
// may expose only a still frame, so reject APNG before passing it to that coder.
fn reject_apng(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(8)).map_err(err)?;
    loop {
        let mut chunk = [0u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            break;
        }
        if &chunk[4..] == b"acTL" {
            return Err("第一階段不支援動畫 PNG".into());
        }
        if &chunk[4..] == b"IDAT" || &chunk[4..] == b"IEND" {
            break;
        }
        let length = u32::from_be_bytes(chunk[..4].try_into().unwrap());
        file.seek(SeekFrom::Current(i64::from(length) + 4))
            .map_err(err)?;
    }
    Ok(())
}

// cICP is a PNG color description independent of ICC. The private conversion
// pipeline cannot yet prove that it survives MIFF and every output coder.
fn png_color_chunks(file: &mut File) -> Result<(Option<[u8; 4]>, bool)> {
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut signature = [0u8; 8];
    file.read_exact(&mut signature).map_err(err)?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return Err("PNG 檔頭無效".into());
    }
    let mut cicp = None;
    let mut iccp = false;
    loop {
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(err)?;
        let length = u32::from_be_bytes(chunk[..4].try_into().unwrap());
        if &chunk[4..] == b"cICP" {
            if length != 4 {
                return Err("PNG cICP 長度無效".into());
            }
            let mut value = [0u8; 4];
            file.read_exact(&mut value).map_err(err)?;
            cicp = Some(value);
            file.seek(SeekFrom::Current(4)).map_err(err)?;
            continue;
        }
        if &chunk[4..] == b"iCCP" {
            iccp = true;
        }
        if &chunk[4..] == b"IDAT" || &chunk[4..] == b"IEND" {
            return Ok((cicp, iccp));
        }
        file.seek(SeekFrom::Current(i64::from(length) + 4))
            .map_err(err)?;
    }
}

/// 目的地 `.tmp*` 的擁有權日誌項目（P1-C）。宣告於 NamedTempFile 之前，使其較晚 drop：
/// 檔案先刪除或發布，項目才釋放。崩潰時留下的項目由下次啟動以 File ID 核對後回收。
/// 檔案系統不提供可靠 File ID（部分 SMB／NAS）時不記錄：只會漏收，不會誤刪。
struct TempRecord(Option<instance_ledger::Entry>);
impl TempRecord {
    fn none() -> Self {
        Self(None)
    }
    fn record(temp: &NamedTempFile) -> Result<Self> {
        let ledger = instance_ledger::global().map_err(err)?;
        Ok(Self(
            ledger
                .record(instance_ledger::Kind::File, temp.as_file(), temp.path())
                .ok(),
        ))
    }
}
impl Drop for TempRecord {
    fn drop(&mut self) {
        if let Some(entry) = self.0.take() {
            entry.release();
        }
    }
}

fn publish(
    mut temp: NamedTempFile,
    dir: &Path,
    stem: &std::ffi::OsStr,
    format: Format,
) -> Result<PathBuf> {
    temp.as_file().sync_all().map_err(err)?;
    for index in 0..10000 {
        check_job(&AtomicBool::new(false))?;
        let mut name = OsString::from(stem);
        if index > 0 {
            name.push(format!(" ({index})"));
        }
        name.push(format!(".{}", format.extension()));
        let target = dir.join(name);
        match temp.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temp = e.file;
            }
            Err(e) => return Err(err(e.error)),
        }
    }
    Err("同名檔案過多，無法產生唯一名稱".into())
}

fn image_stage_error(stage: &str, detail: String) -> String {
    // The UI recognizes this exact value as cancellation, not failure.
    if detail == "已取消" {
        return detail;
    }
    let lower = detail.to_ascii_lowercase();
    let hint = if lower.contains("invalid psd resource block offset") {
        "；ImageMagick 無法讀取 Photoshop 資源區塊，請用原繪圖軟體另存新檔後再試"
    } else if lower.contains("bad_alloc")
        || lower.contains("std::bad_alloc")
        || lower.contains("memory allocation")
        || lower.contains("allocation failed")
        || lower.contains("not enough memory")
        || lower.contains("0xe06d7363")
        || lower.contains("0xc0000017")
    {
        "；引擎無法配置記憶體，可能達到工作 committed-memory 上限或系統可用額度；請關閉其他工作後再試。JXL C++ 例外也可能由記憶體不足引起，原始診斷保留供檢查"
    } else if lower.contains("cache resources exhausted")
        || lower.contains("disk allocation failed")
        || lower.contains("no space left")
    {
        "；請檢查暫存磁碟可用空間；每個引擎行程的磁碟快取上限為 8 GiB"
    } else if lower.contains("time limit") || detail.contains("秒限制") {
        "；單檔總期限及每次引擎執行上限均為 600 秒，請回報圖片尺寸及輸出格式"
    } else {
        ""
    };
    // Keep the original engine diagnostics, including any notices they contain.
    format!("{stage}失敗：{detail}{hint}")
}

#[derive(Clone, Debug)]
pub struct Options {
    pub avif_quality: u8,
    pub webp_lossless: bool,
    pub target_icc: Option<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            avif_quality: 90,
            webp_lossless: true,
            target_icc: None,
        }
    }
}

#[derive(Debug)]
pub struct Verification {
    pub exact: bool,
    pub max_error: f64,
    pub rmse: f64,
    pub input_depth: u32,
    pub output_depth: u32,
    pub icc: bool,
    pub transformed: bool,
    pub color_verified: bool,
    pub color_missing: bool,
    pub warnings: Vec<String>,
}
impl std::fmt::Display for Verification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Build the line from fields and skip empty ones, so no stray separators appear.
        // Plain words instead of "✓": the GUI font may render the glyph as blank space.
        let fields = [
            (if self.exact {
                "pixels exact"
            } else {
                "pixels changed"
            })
            .to_owned(),
            format!("{}→{} bit", self.input_depth, self.output_depth),
            (if self.icc && self.color_verified {
                "ICC preserved"
            } else if self.icc {
                "ICC present, not verified"
            } else {
                "ICC untagged"
            })
            .to_owned(),
            format!("max={:.6}", self.max_error),
            format!("RMSE={:.6}", self.rmse),
            (if self.transformed {
                "compared after requested color/background conversion"
            } else {
                ""
            })
            .to_owned(),
            (if self.color_missing {
                "color information not verified"
            } else {
                ""
            })
            .to_owned(),
        ];
        let line = fields
            .iter()
            .map(|field| field.trim())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        f.write_str(&line)?;
        for warning in &self.warnings {
            write!(f, "\n  警告：{warning}")?;
        }
        Ok(())
    }
}

fn invoke(
    engine: &Path,
    args: &[String],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
) -> Result<()> {
    run(
        engine,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        source,
        output,
        cancel,
    )
}

// Prove absence only from a complete PNG chunk walk, never from ping's ICC list.
// Text/unknown metadata may carry legacy raw profiles, so those require full read.
fn png_profile_hint(source: &mut File) -> Result<(bool, bool)> {
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut signature = [0; 8];
    source.read_exact(&mut signature).map_err(err)?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return Err("PNG 檔頭無效".into());
    }
    let size = source.metadata().map_err(err)?.len();
    let mut at = 8u64;
    let mut present = false;
    let mut ambiguous = false;
    loop {
        if at.checked_add(12).is_none_or(|end| end > size) {
            return Err("PNG 區塊標頭截斷".into());
        }
        source.seek(SeekFrom::Start(at)).map_err(err)?;
        let mut header = [0; 8];
        source.read_exact(&mut header).map_err(err)?;
        let n = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let end = at
            .checked_add(12)
            .and_then(|v| v.checked_add(n))
            .ok_or("PNG 區塊長度溢位")?;
        if end > size {
            return Err("PNG 區塊超出檔案範圍".into());
        }
        match &header[4..] {
            b"iCCP" => present = true,
            b"IHDR" | b"PLTE" | b"IDAT" | b"tRNS" | b"gAMA" | b"cHRM" | b"sRGB" | b"pHYs"
            | b"bKGD" => {}
            b"IEND" => {
                source.seek(SeekFrom::Start(0)).map_err(err)?;
                return Ok((present, ambiguous));
            }
            _ => ambiguous = true,
        }
        at = end;
    }
}
fn icc_model(profile: &mut File) -> Result<[u8; 4]> {
    profile.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut header = [0; 128];
    profile
        .read_exact(&mut header)
        .map_err(|e| format!("ICC 標頭截斷：{e}"))?;
    if &header[36..40] != b"acsp" {
        return Err("ICC 標頭簽章無效，未保存輸出".into());
    }
    profile.seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(header[16..20].try_into().unwrap())
}

fn metadata(
    engine: &Path,
    coder: &str,
    source: &mut File,
    cancel: &AtomicBool,
) -> Result<(u32, bool, u64, u64)> {
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut out = job_file()?;
    let hint = if coder == "png:-" {
        Some(png_profile_hint(source)?)
    } else {
        None
    };
    let mut args = vec![
        "identify",
        "-define",
        "png:preserve-iCCP=true",
        "-format",
        "%z\n%[profiles]\n%w\n%h",
    ];
    // No coder/extension whitelist for ICC. Full profile read is authoritative.
    // The only fast path is structural proof that PNG cannot contain a profile.
    if hint == Some((false, false)) {
        args.push("-ping");
    }
    args.push(coder);
    run(engine, &args, source, &out, cancel)
        .map_err(|e| image_stage_error("讀取位深／ICC (identify)", e))?;
    out.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut text = String::new();
    out.take(4096).read_to_string(&mut text).map_err(err)?;
    let mut lines = text.lines();
    let depth = lines.next().ok_or("缺少位深資訊")?.parse().map_err(err)?;
    let profiles = lines.next().unwrap_or("");
    let has_icc = profiles.split([',', ' ']).any(|s| s == "icc" || s == "icm");
    if hint.is_some_and(|v| v.0) && !has_icc {
        return Err("PNG 有 iCCP，但引擎未能完整讀取 ICC；未保存輸出".into());
    }
    let width = lines.next().ok_or("缺少寬度")?.parse().map_err(err)?;
    let height = lines.next().ok_or("缺少高度")?.parse().map_err(err)?;
    Ok((depth, has_icc, width, height))
}

fn extracted(
    engine: &Path,
    coder: &str,
    source: &mut File,
    kind: &str,
    depth: u32,
    cancel: &AtomicBool,
) -> Result<File> {
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut output = job_file()?;
    let target = format!("{kind}:-");
    let bits = depth.to_string();
    let mut args = vec!["-define", "png:preserve-iCCP=true", coder];
    if kind == "rgba" {
        args.extend(["-alpha", "on", "-depth", &bits, "-endian", "LSB"]);
        if depth == 32 {
            args.extend(["-define", "quantum:format=floating-point"]);
        }
    }
    args.push(&target);
    run(engine, &args, source, &output, cancel)?;
    if output.metadata().map_err(err)?.len() == 0 {
        return Err("驗證資料為空，無法確認畫質".into());
    }
    output.seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(output)
}

fn sample_error(
    a: &mut File,
    b: &mut File,
    depth: u32,
    expected: u64,
    cancel: &AtomicBool,
) -> Result<(bool, f64, f64)> {
    use std::io::BufReader;
    a.seek(SeekFrom::Start(0)).map_err(err)?;
    b.seek(SeekFrom::Start(0)).map_err(err)?;
    let bytes = if depth == 32 { 4 } else { 2 };
    if a.metadata().map_err(err)?.len() != expected * bytes
        || b.metadata().map_err(err)?.len() != expected * bytes
    {
        return Err("驗證尺寸／像素資料長度不符".into());
    }
    let mut a = BufReader::new(a);
    let mut b = BufReader::new(b);
    let mut aa = [0u8; 65536];
    let mut bb = [0u8; 65536];
    let mut maximum = 0f64;
    let mut sum = 0f64;
    let mut exact = true;
    loop {
        check_job(cancel)?;
        // Read exact matching blocks; ordinary files can still return short reads.
        let n = a.read(&mut aa).map_err(err)?;
        if n == 0 {
            break;
        }
        b.read_exact(&mut bb[..n]).map_err(err)?;
        if n % bytes as usize != 0 {
            return Err("驗證樣本長度無效".into());
        }
        for (x, y) in aa[..n]
            .chunks_exact(bytes as usize)
            .zip(bb[..n].chunks_exact(bytes as usize))
        {
            let (x, y) = if depth == 32 {
                (
                    f32::from_le_bytes(x.try_into().unwrap()) as f64,
                    f32::from_le_bytes(y.try_into().unwrap()) as f64,
                )
            } else {
                (
                    u16::from_le_bytes(x.try_into().unwrap()) as f64 / 65535.,
                    u16::from_le_bytes(y.try_into().unwrap()) as f64 / 65535.,
                )
            };
            if !x.is_finite() || !y.is_finite() {
                return Err("驗證遇到非有限浮點像素".into());
            }
            let d = (x - y).abs();
            exact &= d == 0.;
            maximum = maximum.max(d);
            sum += d * d;
        }
    }
    Ok((exact, maximum, (sum / expected.max(1) as f64).sqrt()))
}

#[allow(dead_code)]
pub fn convert(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    // Historical tests/API keep their original lossy WebP behavior.
    let options = Options {
        webp_lossless: false,
        ..Options::default()
    };
    convert_with_options(engine, input, dir, format, &options, cancel).map(|(p, _)| p)
}

pub fn convert_with_options(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<(PathBuf, Verification)> {
    check_job(cancel)?;
    let job = Job::new()?;
    let ((path, mut verification), warnings) = job.finish_with_warnings(
        convert_with_options_inner(engine, input, dir, format, options, cancel),
    )?;
    verification.warnings.extend(
        warnings
            .into_iter()
            .map(|w| format!("輸出已保存 {}；{w}", display_path(&path))),
    );
    Ok((path, verification))
}
fn convert_with_options_inner(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<(PathBuf, Verification)> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    validate_engine(engine)?;
    verify_active_policy(engine, cancel)?;
    let input_format = Format::input(input)?;
    let mut source = File::open(input).map_err(|e| format!("開啟來源失敗：{e}"))?;
    if !source.metadata().map_err(err)?.is_file() {
        return Err("來源不是一般檔案".into());
    }
    if matches!(input_format, Format::Psd) {
        let mut header = [0; 6];
        source
            .read_exact(&mut header)
            .map_err(|e| format!("PSD 檔頭截斷：{e}"))?;
        if &header != b"8BPS\0\x01" {
            return Err("僅支援 PSD v1，不支援 PSB 或無效 PSD".into());
        }
        if let Some(compatible) = psd_compatible_source(&mut source, cancel)? {
            source = compatible;
        }
    }
    if matches!(input_format, Format::Png) {
        reject_apng(&mut source)?;
    }
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut info = job_file()?;
    run(
        engine,
        &[
            "identify",
            "-limit",
            "thread",
            "1",
            "-limit",
            "memory",
            "256MiB",
            "-limit",
            "map",
            "512MiB",
            "-limit",
            "disk",
            "8GiB",
            "-format",
            "%n %w %h %z %[channels]\n",
            // Set explicit -ping after -format for metadata-only inspection.
            "-ping",
            input_format.input_coder(),
        ],
        &source,
        &info,
        cancel,
    )
    .map_err(|e| image_stage_error("讀取圖片資訊 (identify)", e))?;
    info.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut frames = String::new();
    info.take(256).read_to_string(&mut frames).map_err(err)?;
    let fields: Vec<_> = frames.split_whitespace().collect();
    if fields.len() != 6 || fields[0] != "1" {
        return Err("僅支援靜態單張圖片（不支援多頁 TIFF 或動畫）".into());
    }
    let width = fields[1].parse::<u64>().map_err(err)?;
    let height = fields[2].parse::<u64>().map_err(err)?;
    let depth = fields[3].parse::<u64>().map_err(err)?;
    let channels = fields[5].parse::<f64>().map_err(err)?;
    if !channels.is_finite() || channels < 1.0 || channels > 64.0 || channels.fract() != 0.0 {
        return Err("來源通道數無效".into());
    }
    let channels = channels as u64;
    JOB_IMAGE.with(|v| v.set((width.saturating_mul(height), depth, channels)));
    if width.saturating_mul(height) > 400_000_000 {
        return Err("圖片超過 400 MP（4 億像素）上限".into());
    }
    let reservations = controlled_temp::preflight(
        &job_dir(),
        dir,
        width.saturating_mul(height),
        source.metadata().map_err(err)?.len(),
    )
    .map_err(|e| format!("磁碟資源預檢失敗：{e}"))?;
    JOB_RESERVATIONS.with(|r| r.borrow_mut().extend(reservations));
    if matches!(format, Format::Psd) && (width > 30000 || height > 30000) {
        return Err("PSD 尺寸上限為每邊 30000 像素；不輸出 PSB".into());
    }
    if !(1..=100).contains(&options.avif_quality) {
        return Err("AVIF 品質須為 1–100".into());
    }
    let (input_depth, source_icc, _, _) =
        metadata(engine, input_format.input_coder(), &mut source, cancel)?;
    let mut source_profile_hash = None;
    let mut effective_icc_model = None;
    if source_icc {
        let mut profile = extracted(
            engine,
            input_format.input_coder(),
            &mut source,
            "icc",
            16,
            cancel,
        )?;
        effective_icc_model = Some(icc_model(&mut profile)?);
        source_profile_hash = Some(sha256(&mut profile)?);
    }
    // JXL exposes a decoder ICC representation, not proof that ICC bytes were
    // embedded. The bundled coder discards this provenance flag. Keep its color
    // definition and all verification requirements even when origin is unknown.
    let source_icc_model = effective_icc_model;
    if matches!(input_format, Format::Png) {
        let (cicp, iccp) = png_color_chunks(&mut source)?;
        if cicp.is_some() && (!iccp || !source_icc) {
            return Err("此 PNG 僅有 cICP、沒有原始 iCCP；目前無法驗證轉檔後完整保留色彩資訊，未保存輸出。請使用已嵌入正確 ICC 的來源".into());
        }
    }
    let mut color_missing = false;
    let mut avif_output_cicp = String::from("heic:cicp=1/13/6/1");
    // CICP-only HDR/wide-gamut AVIF has no ICC for the proven preservation path.
    // Refuse rather than silently relabeling PQ/HLG samples as sRGB.
    if matches!(input_format, Format::Avif) && !source_icc {
        source.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut out = job_file()?;
        run(
            engine,
            &[
                "identify",
                "-format",
                "%[heic:cicp]",
                "-ping",
                input_format.input_coder(),
            ],
            &source,
            &out,
            cancel,
        )?;
        out.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut text = String::new();
        out.take(256).read_to_string(&mut text).map_err(err)?;
        if text.trim().is_empty() {
            color_missing = true;
        } else {
            validate_avif_cicp(&text)?;
            let transfer = text.trim().split('/').nth(1).ok_or("CICP transfer 缺漏")?;
            avif_output_cicp = format!("heic:cicp=1/{transfer}/6/1");
        }
    }
    if options.webp_lossless && matches!(format, Format::Webp) && input_depth > 8 {
        return Err("WebP 僅支援 8-bit；原圖高於 8-bit，無法真正無損。請選 PNG/TIFF/JXL，或關閉 WebP 無損以允許降低位深".into());
    }
    if input_depth > 16 && !matches!(format, Format::Jxl | Format::Tiff) {
        return Err("超過 16-bit／浮點來源請使用 JXL 或 TIFF，避免靜默截斷精度".into());
    }
    let destination = dir
        .canonicalize()
        .map_err(|e| format!("輸出目錄解析失敗：{e}"))?;
    let mut intermediate = job_file()?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut args: Vec<String> = [
        "-limit",
        "thread",
        "1",
        "-limit",
        "memory",
        "256MiB",
        "-limit",
        "map",
        "512MiB",
        "-limit",
        "disk",
        "8GiB",
        "-define",
        "png:preserve-iCCP=true",
        input_format.input_coder(),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if matches!(input_format, Format::Jpeg) {
        // Apply EXIF Orientation 1..8 once, before writing a new container.
        // This also resets the orientation tag, preventing a second rotation.
        args.push("-auto-orient".into());
    }
    // Snapshot user ICC into a safe generated filename; never interpret a user path as a coder/option.
    let mut profile_snapshot = None;
    let mut target_profile_hash = None;
    if let Some(path) = &options.target_icc {
        if !source_icc {
            return Err("來源沒有 ICC；無法可靠判斷原始色域，請先為來源嵌入正確 ICC".into());
        }
        let mut profile = tempfile::Builder::new()
            .rand_bytes(16)
            .prefix("imb-c-profile-")
            .suffix(".icc")
            .tempfile_in(job_cwd())
            .map_err(err)?;
        let f = File::open(path).map_err(err)?;
        if f.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
            return Err("ICC 大於 16 MiB".into());
        }
        let mut data = Vec::new();
        f.take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(err)?;
        if data.len() > 16 * 1024 * 1024
            || data.len() < 132
            || &data[36..40] != b"acsp"
            || u32::from_be_bytes(data[..4].try_into().unwrap()) as usize != data.len()
        {
            return Err("ICC 標頭或長度無效".into());
        }
        write_job_bytes(profile.as_file_mut(), &data)?;
        effective_icc_model = Some(data[16..20].try_into().unwrap());
        target_profile_hash = Some(sha256(profile.as_file_mut())?);
        args.extend([
            "-profile".into(),
            profile
                .path()
                .file_name()
                .ok_or("ICC 暫存檔名無效")?
                .to_string_lossy()
                .into_owned(),
        ]);
        profile_snapshot = Some(profile);
    }
    if matches!(format, Format::Webp) && effective_icc_model.is_some_and(|model| model != *b"RGB ")
    {
        return Err(color_diagnostics::profile(
            &effective_icc_model.unwrap(),
            matches!(input_format, Format::Jxl) && options.target_icc.is_none(),
        ));
    }
    if matches!(format, Format::Jpeg) {
        args.extend(
            ["-background", "white", "-alpha", "remove", "-alpha", "off"].map(String::from),
        );
    }
    if matches!(format, Format::Psd) {
        args.extend(["+profile", "8bim"].map(String::from));
    }
    args.extend(
        [
            "-evaluate",
            "Add",
            "0",
            "-define",
            "quantum:format=floating-point",
            "-depth",
            "32",
            "miff:-",
        ]
        .map(String::from),
    );
    invoke(engine, &args, &source, &intermediate, cancel)
        .map_err(|e| image_stage_error("圖片轉換 (convert)／高精度中繼", e))?;
    let (_, intermediate_icc, expected_width, expected_height) =
        metadata(engine, "miff:-", &mut intermediate, cancel)?;
    if intermediate_icc != source_icc {
        return Err("中繼 ICC 遺失或意外新增，未保存輸出".into());
    }
    if let Some(expected) = &target_profile_hash {
        let mut actual = extracted(engine, "miff:-", &mut intermediate, "icc", 16, cancel)?;
        if &sha256(&mut actual)? != expected {
            return Err("目標 ICC 未正確套用，未保存輸出".into());
        }
    }
    drop(profile_snapshot);
    if intermediate.metadata().map_err(err)?.len() == 0 {
        return Err("中繼輸出為空".into());
    }
    intermediate.seek(SeekFrom::Start(0)).map_err(err)?;
    #[allow(unused_assignments)] // 只為 drop 順序先宣告
    let mut _temp_record = TempRecord::none();
    let mut temp = NamedTempFile::new_in(&destination).map_err(err)?;
    _temp_record = TempRecord::record(&temp)?;
    // ICC conversion can produce fractional samples from 8-bit input.
    // Quantize only to 16-bit for formats that support it, matching verification.
    let depth = if input_depth > 16 {
        "32"
    } else if input_depth > 8
        || (options.target_icc.is_some()
            && matches!(
                format,
                Format::Png | Format::Tiff | Format::Jxl | Format::Psd
            ))
        || (matches!(input_format, Format::Psd)
            && matches!(format, Format::Png | Format::Tiff | Format::Jxl))
    {
        "16"
    } else {
        "8"
    };
    let q = options.avif_quality.to_string();
    let mut args = vec![
        "-limit", "thread", "1", "-limit", "memory", "256MiB", "-limit", "map", "512MiB", "-limit",
        "disk", "8GiB", "miff:-", "-depth", depth,
    ];
    if input_depth > 16 {
        args.extend(["-define", "quantum:format=floating-point"]);
    }
    if matches!(format, Format::Jpeg) {
        args.extend(["-quality", "90"]);
    }
    if matches!(format, Format::Webp) {
        if options.webp_lossless {
            args.extend([
                "-quality",
                "100",
                "-define",
                "webp:lossless=true",
                "-define",
                "webp:exact=true",
            ]);
        } else {
            args.extend(["-quality", "90", "-define", "webp:lossless=false"]);
        }
    }
    if matches!(format, Format::Avif) {
        // The bundled encoder otherwise omits nclx for ordinary unprofiled SDR.
        // Record its existing sRGB/BT.601 full-range encoding explicitly, so our
        // own output also satisfies the strict CICP-only input validation.
        if !source_icc && !color_missing {
            args.extend(["-define", &avif_output_cicp]);
        }
        args.extend([
            "-quality",
            &q,
            "-define",
            "heic:speed=6",
            "-define",
            "heic:chroma=444",
        ]);
    }
    if matches!(format, Format::Png) {
        args.extend(["-define", "png:preserve-iCCP=true"]);
        // An RGB ICC must not be written into grayscale PNG storage. Keep all
        // samples (including alpha) and use an RGB-compatible PNG color type.
        if effective_icc_model == Some(*b"RGB ") {
            args.extend(["-define", "png:color-type=6"]);
        }
        if depth == "16" {
            args.extend(["-define", "png:bit-depth=16"]);
        }
    }
    if matches!(format, Format::Tiff) {
        args.extend(["-compress", "Zip", "-define", "tiff:alpha=unassociated"]);
    }
    if matches!(format, Format::Jxl) {
        args.extend(["-define", "jxl:distance=0", "-quality", "100"]);
    }
    if matches!(format, Format::Psd) {
        args.extend(["-define", "psd:write-layers=true"]);
    }
    args.push(format.coder());
    run(engine, &args, &intermediate, temp.as_file(), cancel)
        .map_err(|e| image_stage_error("圖片轉換 (convert)", e))?;
    if temp.as_file().metadata().map_err(err)?.len() == 0 {
        return Err("轉換輸出為空".into());
    }
    let (output_depth, output_icc, output_width, output_height) =
        metadata(engine, format.input_coder(), temp.as_file_mut(), cancel)?;
    if (output_width, output_height) != (expected_width, expected_height) {
        return Err("驗證輸出寬高不符，未保存輸出".into());
    }
    let transformed = options.target_icc.is_some()
        || matches!(format, Format::Jpeg)
        || matches!(input_format, Format::Jpeg);
    let mut reference = if transformed {
        intermediate.try_clone().map_err(err)?
    } else {
        source.try_clone().map_err(err)?
    };
    let reference_coder = if transformed {
        "miff:-"
    } else {
        input_format.input_coder()
    };
    let expected_icc = source_icc; // Original authoritative profile, not a possibly altered intermediate.
    let expected_profile_hash = target_profile_hash.or(source_profile_hash);
    // libjxl's decoder may expose an ICC representation of its internal color
    // encoding even when the source and encoded JXL contain no ICC profile.
    let jxl_decoder_profile = matches!(format, Format::Jxl) && !expected_icc && output_icc;
    if expected_icc != output_icc && !jxl_decoder_profile {
        return Err(if expected_icc {
            "ICC 描述檔遺失，未保存輸出".into()
        } else {
            "輸出意外新增 ICC 描述檔，未保存輸出".into()
        });
    }
    if expected_icc {
        let mut b = extracted(
            engine,
            format.input_coder(),
            temp.as_file_mut(),
            "icc",
            16,
            cancel,
        )?;
        if Some(sha256(&mut b)?) != expected_profile_hash {
            return Err("ICC 描述檔不一致，未保存輸出".into());
        }
    }
    let check_depth = if input_depth > 16 { 32 } else { 16 };
    let mut a = extracted(
        engine,
        reference_coder,
        &mut reference,
        "rgba",
        check_depth,
        cancel,
    )?;
    let mut b = extracted(
        engine,
        format.input_coder(),
        temp.as_file_mut(),
        "rgba",
        check_depth,
        cancel,
    )?;
    let (exact, max_error, rmse) = sample_error(
        &mut a,
        &mut b,
        check_depth,
        expected_width * expected_height * 4,
        cancel,
    )?;
    let lossless = matches!(format, Format::Png | Format::Tiff | Format::Jxl)
        || (matches!(format, Format::Webp) && options.webp_lossless);
    if lossless && !exact {
        if matches!(format, Format::Webp)
            && options.target_icc.is_some()
            && source_icc_model.is_some_and(|model| model != *b"RGB ")
        {
            return Err(color_diagnostics::precision(max_error));
        }
        return Err(format!("無損像素驗證失敗 max={max_error:.8}，未保存輸出"));
    }
    if lossless && input_depth > 8 && output_depth < input_depth {
        return Err("輸出位深降低，未保存無損輸出".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    check_job(cancel)?;
    let result = publish(
        temp,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        format,
    )?;
    Ok((
        result,
        Verification {
            exact,
            max_error,
            rmse,
            input_depth,
            output_depth,
            icc: expected_icc,
            transformed,
            color_verified: expected_icc,
            color_missing,
            warnings: if color_missing {
                vec!["來源 AVIF 缺少 ICC／CICP；沿用解碼器行為完成轉檔，僅驗證樣本數值，未驗證色域／傳輸特性；請保留原檔".into()]
            } else {
                Vec::new()
            },
        },
    ))
}

pub fn sha256(file: &mut File) -> Result<String> {
    use sha2::{Digest, Sha256};
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        check_job(&AtomicBool::new(false))?;
        let count = file.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(format!("{:x}", hash.finalize()))
}

// JPEG reconstruction is deliberately separate from ordinary pixel conversion.
// A source snapshot and a private output file prevent races with destination names.
#[allow(dead_code)] // Compatibility entry point used by existing verification binaries.
pub fn jpeg_reconstruction(
    tools: &Path,
    input: &Path,
    dir: &Path,
    wrap: bool,
    reference: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, String, bool)> {
    let (path, hash, verified, warnings) =
        jpeg_reconstruction_with_warnings(tools, input, dir, wrap, reference, cancel)?;
    for warning in warnings {
        eprintln!("警告：{warning}");
    }
    Ok((path, hash, verified))
}
pub fn jpeg_reconstruction_with_warnings(
    tools: &Path,
    input: &Path,
    dir: &Path,
    wrap: bool,
    reference: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, String, bool, Vec<String>)> {
    check_job(cancel)?;
    let job = Job::new()?;
    let ((path, hash, verified), warnings) = job.finish_with_warnings(
        jpeg_reconstruction_inner(tools, input, dir, wrap, reference, cancel),
    )?;
    let warnings = warnings
        .into_iter()
        .map(|w| format!("輸出已保存 {}；{w}", display_path(&path)))
        .collect();
    Ok((path, hash, verified, warnings))
}
fn jpeg_reconstruction_inner(
    tools: &Path,
    input: &Path,
    dir: &Path,
    wrap: bool,
    reference: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, String, bool)> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    if !tools.is_absolute() {
        return Err("JXL 工具路徑必須為絕對路徑".into());
    }
    let ext = input
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if (wrap && ext != "jpg" && ext != "jpeg") || (!wrap && ext != "jxl") {
        return Err("此工具僅接受 JPEG → JXL 封裝或 JXL → 原始 JPEG 還原".into());
    }
    let mut source = job_file()?;
    let mut original = File::open(input).map_err(err)?;
    let length = original.metadata().map_err(err)?.len();
    if length > engine_limits::OUTPUT_LIMIT {
        return Err("JPEG ↔ JXL 來源副本超過 16 GiB 上限".into());
    }
    copy_bytes(&mut original, &mut source, length, cancel)?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let destination = dir.canonicalize().map_err(err)?;
    #[allow(unused_assignments)] // 只為 drop 順序先宣告
    let mut _output_record = TempRecord::none();
    let mut output = NamedTempFile::new_in(&destination).map_err(err)?;
    _output_record = TempRecord::record(&output)?;
    let cjxl = tools.join("cjxl.exe");
    let djxl = tools.join("djxl.exe");
    let hash;
    let verified;
    if wrap {
        hash = sha256(&mut source)?;
        run(
            &cjxl,
            // libjxl 0.13 預設對 4:4:4／灰階 JPEG 啟用 LF smoothing；libjxl 0.12 以前的 djxl
            // 會以退出碼 0 還原出不同位元組。明確關閉以保持既有輸出的跨版本無損還原。
            &[
                "-",
                "-",
                "--lossless_jpeg=1",
                "--jpeg_reconstruction_lfs=0",
                "--num_threads=1",
            ],
            &source,
            output.as_file(),
            cancel,
        )?;
        output.as_file_mut().seek(SeekFrom::Start(0)).map_err(err)?;
        let mut restored = job_file()?;
        run(
            &djxl,
            &[
                "-",
                "-",
                "--output_format=jpg",
                "--reconstruct_jpeg",
                "--num_threads=1",
            ],
            output.as_file(),
            &restored,
            cancel,
        )?;
        if sha256(&mut restored)? != hash {
            return Err("JPEG 往返 SHA-256 不一致，未保存輸出".into());
        }
        verified = true;
    } else {
        run(
            &djxl,
            &[
                "-",
                "-",
                "--output_format=jpg",
                "--reconstruct_jpeg",
                "--num_threads=1",
            ],
            &source,
            output.as_file(),
            cancel,
        )?;
        hash = sha256(output.as_file_mut())?;
        if let Some(path) = reference {
            if sha256(&mut File::open(path).map_err(err)?)? != hash {
                return Err("與參考 JPEG 的 SHA-256 不一致，未保存輸出".into());
            }
            verified = true;
        } else {
            verified = false;
        }
    }
    if output.as_file().metadata().map_err(err)?.len() == 0 {
        return Err("輸出為空".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    check_job(cancel)?;
    let result = publish(
        output,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        if wrap { Format::Jxl } else { Format::Jpeg },
    )?;
    Ok((result, hash, verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn verification_line_skips_empty_fields() {
        let base = || Verification {
            exact: false,
            max_error: 0.5,
            rmse: 0.25,
            input_depth: 8,
            output_depth: 8,
            icc: true,
            transformed: false,
            color_verified: true,
            color_missing: false,
            warnings: Vec::new(),
        };
        let line = |v: Verification| v.to_string();
        let preserved = line(base());
        assert_eq!(
            preserved,
            "pixels changed; 8→8 bit; ICC preserved; max=0.500000; RMSE=0.250000"
        );
        let untagged = line(Verification {
            exact: true,
            icc: false,
            color_verified: false,
            color_missing: true,
            ..base()
        });
        assert_eq!(
            untagged,
            "pixels exact; 8→8 bit; ICC untagged; max=0.500000; RMSE=0.250000; color information not verified"
        );
        let converted = line(Verification {
            color_verified: false,
            transformed: true,
            warnings: vec!["w".into()],
            ..base()
        });
        assert!(converted.starts_with(
            "pixels changed; 8→8 bit; ICC present, not verified; max=0.500000; RMSE=0.250000; compared after requested color/background conversion\n  警告：w"
        ));
        for text in [preserved, untagged, converted] {
            let first = text.lines().next().unwrap();
            assert!(!first.contains("  ") && !first.contains(" ;") && !first.contains(";;"));
            assert!(!first.ends_with(';') && !first.contains('✓'), "{first}");
        }
    }

    #[test]
    fn psd_normalization_only_removes_empty_name_resources() {
        fn resource(id: u16, name: &[u8], data: &[u8]) -> Vec<u8> {
            let mut v = b"8BIM".to_vec();
            v.extend(id.to_be_bytes());
            v.push(name.len() as u8);
            v.extend(name);
            if (name.len() + 1) % 2 != 0 {
                v.push(0);
            }
            v.extend((data.len() as u32).to_be_bytes());
            v.extend(data);
            if data.len() % 2 != 0 {
                v.push(0);
            }
            v
        }
        let empty_a = resource(1006, b"", b"");
        let icc = resource(1039, b"icc", b"ICC payload exactly preserved");
        let exif = resource(1058, b"exif", b"EXIF bytes");
        let nonempty_name = resource(1045, b"", b"nonempty alpha names");
        let unknown = resource(4095, b"", b"");
        let empty_b = resource(1045, b"", b"");
        let kept = [
            icc.clone(),
            exif.clone(),
            nonempty_name.clone(),
            unknown.clone(),
        ]
        .concat();
        let resources = [empty_a, icc, exif, nonempty_name, unknown, empty_b].concat();
        let mut header = b"8BPS\0\x01".to_vec();
        header.resize(26, 0);
        header.extend(3u32.to_be_bytes());
        header.extend(b"RGB");
        let tail = b"layer/mask/composite data byte for byte";
        let original = [
            header.clone(),
            (resources.len() as u32).to_be_bytes().to_vec(),
            resources,
            tail.to_vec(),
        ]
        .concat();
        let expected = [
            header.clone(),
            (kept.len() as u32).to_be_bytes().to_vec(),
            kept,
            tail.to_vec(),
        ]
        .concat();
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&original).unwrap();
        let mut normalized = psd_compatible_source(&mut file, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        normalized.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        file.seek(SeekFrom::Start(0)).unwrap();
        bytes.clear();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, original);
        assert!(
            psd_compatible_source(&mut normalized, &AtomicBool::new(false))
                .unwrap()
                .is_none()
        );
        file.set_len((header.len() + 5) as u64).unwrap();
        assert!(
            psd_compatible_source(&mut file, &AtomicBool::new(false))
                .unwrap_err()
                .contains("範圍")
        );
    }
    #[test]
    fn strict_cicp_fields_and_supported_sdr() {
        for text in ["1/13/6/1", "1/1/1/0", "1/13/0/1"] {
            validate_avif_cicp(text).unwrap();
        }
        for text in [
            "",
            "1/13",
            "1/x/13/6/1",
            "1//6/1",
            "9/16/9/1",
            "9/18/9/1",
            "12/13/6/1",
            "1/2/6/1",
            "1/13/99/1",
            "1/13/6/2",
            "1/13/6/1/0",
        ] {
            assert!(validate_avif_cicp(text).is_err(), "{text}");
        }
    }
    #[test]
    fn private_engine_rejects_relative_missing_and_wrong_policy() {
        assert!(validate_engine(Path::new("ImageMagick/magick.exe")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("magick.exe");
        fs::write(&engine, b"must not execute").unwrap();
        assert!(validate_engine(&engine).unwrap_err().contains("policy"));
        fs::write(dir.path().join("policy.xml"), b"wrong").unwrap();
        assert!(validate_engine(&engine).unwrap_err().contains("policy"));
        fs::write(
            dir.path().join("policy.xml"),
            include_bytes!("../assets/policy-v13.xml"),
        )
        .unwrap();
        assert!(validate_engine(&engine).unwrap_err().contains("coder"));
    }
    #[test]
    fn jxl_locator_errors_are_recoverable() {
        assert!(
            locate_jxl_tools(Err(std::io::Error::other("injected current_exe failure")))
                .unwrap_err()
                .contains("injected")
        );
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("imb-c.exe");
        assert!(
            locate_jxl_tools(Ok(executable.clone()))
                .unwrap_err()
                .contains("缺少")
        );
        fs::create_dir(dir.path().join("JXL")).unwrap();
        for file in ["cjxl.exe", "djxl.exe"] {
            fs::write(dir.path().join("JXL").join(file), []).unwrap();
        }
        assert_eq!(
            locate_jxl_tools(Ok(executable)).unwrap(),
            dir.path().join("JXL")
        );
    }
    #[test]
    fn job_deadline_stops_later_stages_and_preserves_cancellation() {
        let job = Job::new().unwrap();
        let root = job_dir();
        let input = job_file().unwrap();
        let output = job_file().unwrap();
        JOB.with(|j| {
            j.borrow_mut().as_mut().unwrap().1 = Instant::now() - Duration::from_millis(1)
        });
        let off = AtomicBool::new(false);
        let e = run(Path::new("must-not-launch.exe"), &[], &input, &output, &off).unwrap_err();
        assert!(e.contains("總期限"), "{e}");
        assert_eq!(check_job(&AtomicBool::new(true)).unwrap_err(), "已取消");
        drop((input, output));
        assert!(job.finish::<()>(Err(e)).is_err());
        assert!(!root.exists());
    }
    #[test]
    fn job_retains_parent_when_child_stop_is_unknown() {
        let job = Job::new().unwrap();
        let root = job_dir();
        JOB.with(|j| j.borrow_mut().as_mut().unwrap().2 = true);
        assert!(
            job.finish::<()>(Err("injected wait failure".into()))
                .unwrap_err()
                .contains("保留")
        );
        assert!(root.exists());
        // This fixture never started a child; deleting this exact test directory is safe.
        fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn psd_resource_error_keeps_original_and_adds_actionable_hint() {
        let message = image_stage_error(
            "identify",
            "exit 1: Invalid PSD resource block offset".into(),
        );
        assert!(message.contains("Invalid PSD resource block offset"));
        assert!(message.contains("另存新檔"));
    }
    #[cfg(windows)]
    #[test]
    fn total_deadline_includes_multiple_child_stages() {
        let job = Job::new().unwrap();
        let root = job_dir();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let input = job_file().unwrap();
        let output = job_file().unwrap();
        let off = AtomicBool::new(false);
        // Each child has a 10-second allowance, but they share a 3-second job deadline.
        JOB.with(|j| j.borrow_mut().as_mut().unwrap().1 = Instant::now() + Duration::from_secs(3));
        run_with_timeout(
            &shell,
            &["-NoProfile", "-Command", "Start-Sleep -Milliseconds 500"],
            &input,
            &output,
            &off,
            Duration::from_secs(10),
        )
        .unwrap();
        let e = run_with_timeout(
            &shell,
            &["-NoProfile", "-Command", "Start-Sleep -Seconds 5"],
            &input,
            &output,
            &off,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("總期限"), "{e}");
        assert!(
            run(&shell, &[], &input, &output, &off)
                .unwrap_err()
                .contains("總期限")
        );
        drop((input, output));
        job.finish(Ok(())).unwrap();
        assert!(!root.exists());
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn private_policy_is_applied_and_forbidden_coders_are_denied() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let job = Job::new().unwrap();
        let off = AtomicBool::new(false);
        let mut source = job_file().unwrap();
        let mut out = job_file().unwrap();
        run(&engine, &["-list", "policy"], &source, &out, &off).unwrap();
        out.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        out.read_to_string(&mut text).unwrap();
        assert!(
            text.contains("policy.xml") && text.contains("Delegate") && text.contains("8GiB"),
            "{text}"
        );
        assert!(!text.contains("hostile-policy"), "{text}");
        source
            .write_all(b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>")
            .unwrap();
        source.seek(SeekFrom::Start(0)).unwrap();
        out.set_len(0).unwrap();
        out.seek(SeekFrom::Start(0)).unwrap();
        let error = run(&engine, &["svg:-", "png:-"], &source, &out, &off).unwrap_err();
        assert!(
            error.contains("security policy") || error.contains("not authorized"),
            "{error}"
        );
        assert_eq!(out.metadata().unwrap().len(), 0);
        drop((source, out));
        job.finish(Ok(())).unwrap();
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn special_temp_root_pixel_icc_and_target_profile() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let special = dir.path().join("中文 空白 %03d [1]");
        fs::create_dir(&special).unwrap();
        let src = dir.path().join("source.png");
        fs::write(
            &src,
            include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
        )
        .unwrap();
        let profile =
            PathBuf::from(r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm");
        let options = Options {
            target_icc: Some(profile),
            ..Default::default()
        };
        let off = AtomicBool::new(false);
        let job = Job::new_in(&special).unwrap();
        let root = job_dir();
        let (_, verification) = job
            .finish(convert_with_options_inner(
                &engine,
                &src,
                dir.path(),
                Format::Png,
                &options,
                &off,
            ))
            .unwrap();
        assert!(verification.exact && verification.icc && verification.transformed);
        assert!(!root.exists());
        assert_eq!(fs::read_dir(&special).unwrap().count(), 0);
    }
    #[test]
    #[ignore = "Read-only real PSD sample; set IMBC_TEST_ENGINE and IMBC_TEST_ASSETS"]
    fn real_csp_psd_conversion_and_source_preservation() {
        let Some(assets) = std::env::var_os("IMBC_TEST_ASSETS") else {
            eprintln!(
                "skipped real_csp_psd_conversion_and_source_preservation: \
                 set IMBC_TEST_ASSETS to the folder containing rukamizugi.psd"
            );
            return;
        };
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let input = PathBuf::from(assets).join("rukamizugi.psd");
        let mut f = File::open(&input).unwrap();
        let before = sha256(&mut f).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = convert_with_options(
            &engine,
            &input,
            dir.path(),
            Format::Jxl,
            &Options::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(before, sha256(&mut f).unwrap());
        let (_, v) = result.unwrap();
        assert!(v.exact && v.icc);
        println!("real PSD source SHA256={before}; {v}");
    }
    #[test]
    fn verification_detects_changed_empty_and_float_samples() {
        use std::io::Write;
        let mut a = tempfile::tempfile().unwrap();
        let mut b = tempfile::tempfile().unwrap();
        a.write_all(&[0, 0, 255, 255]).unwrap();
        b.write_all(&[0, 0, 0, 0]).unwrap();
        let (exact, max, rmse) =
            sample_error(&mut a, &mut b, 16, 2, &AtomicBool::new(false)).unwrap();
        assert!(!exact);
        assert_eq!(max, 1.);
        assert!((rmse - 0.5f64.sqrt()).abs() < 1e-10);
        assert!(sample_error(&mut a, &mut b, 16, 3, &AtomicBool::new(false)).is_err());
        a.set_len(0).unwrap();
        b.set_len(0).unwrap();
        a.seek(SeekFrom::Start(0)).unwrap();
        b.seek(SeekFrom::Start(0)).unwrap();
        for x in [-0.25f32, 2.0] {
            a.write_all(&x.to_le_bytes()).unwrap();
            b.write_all(&x.to_le_bytes()).unwrap();
        }
        assert!(
            sample_error(&mut a, &mut b, 32, 2, &AtomicBool::new(false))
                .unwrap()
                .0
        );
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn v13_float_precision() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("float.tiff");
        fs::write(&src, include_bytes!("../tests/fixtures/v13-float32.tiff")).unwrap();
        for format in [Format::Tiff, Format::Jxl] {
            let (_, v) = convert_with_options(
                &engine,
                &src,
                dir.path(),
                format,
                &Options::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(v.exact && v.input_depth == 32 && v.output_depth == 32);
        }
        assert!(
            convert_with_options(
                &engine,
                &src,
                dir.path(),
                Format::Png,
                &Options::default(),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("浮點")
        );
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn v13_cicp_only_hdr_rejected() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("cicp.avif");
        fs::write(&src, include_bytes!("../tests/fixtures/v13-cicp-pq.avif")).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert!(
            convert_with_options(
                &engine,
                &src,
                dir.path(),
                Format::Jxl,
                &Options::default(),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("CICP")
        );
        assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
        let png = dir.path().join("sdr.png");
        fs::write(&png, include_bytes!("../tests/fixtures/v13-hidden.png")).unwrap();
        let (avif, _) = convert_with_options(
            &engine,
            &png,
            dir.path(),
            Format::Avif,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let (_, v) = convert_with_options(
            &engine,
            &avif,
            dir.path(),
            Format::Png,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(v.exact);
    }
    #[test]
    fn png_cicp_parser_finds_color_metadata() {
        let mut f = tempfile::tempfile().unwrap();

        // PNG signature
        f.write_all(b"\x89PNG\r\n\x1a\n").unwrap();

        // cICP chunk
        f.write_all(&4u32.to_be_bytes()).unwrap();
        f.write_all(b"cICP").unwrap();
        f.write_all(&[9, 16, 0, 1]).unwrap();
        f.write_all(&0u32.to_be_bytes()).unwrap();

        // IEND chunk
        f.write_all(&0u32.to_be_bytes()).unwrap();
        f.write_all(b"IEND").unwrap();
        f.write_all(&0xAE426082u32.to_be_bytes()).unwrap();

        assert_eq!(
            png_color_chunks(&mut f).unwrap(),
            (Some([9, 16, 0, 1]), false)
        );
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn cicp_only_hdr_png_is_rejected_without_output() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("pq.png");
        fs::write(
            &input,
            include_bytes!("../tests/fixtures/v13-cicp-only-pq.png"),
        )
        .unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        for format in [Format::Png, Format::Tiff, Format::Jxl, Format::Avif] {
            let error = convert_with_options(
                &engine,
                &input,
                dir.path(),
                format,
                &Options::default(),
                &AtomicBool::new(false),
            )
            .unwrap_err();
            assert!(error.contains("cICP"), "{error}");
            assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
        }
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn jpeg_exif_orientations_to_avif() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let original = include_bytes!("../tests/fixtures/acceptance-v13/baseline-metadata.jpg");
        for orientation in 1..=8u8 {
            let mut bytes = original.to_vec();
            let tag = bytes
                .windows(4)
                .position(|x| x == [0x12, 0x01, 0x03, 0x00])
                .unwrap();
            bytes[tag + 8] = orientation;
            let input = dir.path().join(format!("orientation-{orientation}.jpg"));
            fs::write(&input, bytes).unwrap();
            let (avif, v) = convert_with_options(
                &engine,
                &input,
                dir.path(),
                Format::Avif,
                &Options::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
            let mut output = File::open(avif).unwrap();
            let (_, _, w, h) =
                metadata(&engine, "avif:-", &mut output, &AtomicBool::new(false)).unwrap();
            assert_eq!((w, h), if orientation >= 5 { (13, 17) } else { (17, 13) });
            assert!(v.max_error < 0.15, "orientation {orientation}: {v}");
            let mut raw = File::open(&input).unwrap();
            let mut a = extracted(
                &engine,
                "jpeg:-",
                &mut raw,
                "rgba",
                16,
                &AtomicBool::new(false),
            )
            .unwrap();
            let mut b = extracted(
                &engine,
                "avif:-",
                &mut output,
                "rgba",
                16,
                &AtomicBool::new(false),
            )
            .unwrap();
            let mut source_pixels = Vec::new();
            let mut output_pixels = Vec::new();
            a.read_to_end(&mut source_pixels).unwrap();
            b.read_to_end(&mut output_pixels).unwrap();
            let mut error = 0u64;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let (sx, sy) = match orientation {
                        1 => (x, y),
                        2 => (16 - x, y),
                        3 => (16 - x, 12 - y),
                        4 => (x, 12 - y),
                        5 => (y, x),
                        6 => (y, 12 - x),
                        7 => (16 - y, 12 - x),
                        8 => (16 - y, x),
                        _ => unreachable!(),
                    };
                    for channel in 0..3 {
                        let source_index = (sy * 17 + sx) * 8 + channel * 2;
                        let output_index = (y * w as usize + x) * 8 + channel * 2;
                        let from = u16::from_le_bytes(
                            source_pixels[source_index..source_index + 2]
                                .try_into()
                                .unwrap(),
                        );
                        let to = u16::from_le_bytes(
                            output_pixels[output_index..output_index + 2]
                                .try_into()
                                .unwrap(),
                        );
                        error += from.abs_diff(to) as u64;
                    }
                }
            }
            assert!(
                error as f64 / (w * h * 3) as f64 / 65535. < 0.15,
                "orientation {orientation}: pixel positions differ"
            );
        }
    }
    #[test]
    fn stage_diagnostics_preserve_details_and_cancellation() {
        assert_eq!(image_stage_error("identify", "已取消".into()), "已取消");
        let original = "cache resources exhausted; License: unchanged notice";
        let message = image_stage_error("identify", original.into());
        assert!(message.contains(original));
        assert!(message.contains("8 GiB"));
        assert!(image_stage_error("convert", "超過單次執行 600 秒限制".into()).contains("600 秒"));
    }

    #[test]
    #[ignore = "Requires v1.2.1 candidate engine; set IMBC_TEST_ENGINE"]
    fn v121_small_formats_and_failure_stages() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("透明圖 [1] %03d.png");
        let bytes = include_bytes!("../tests/fixtures/rgba-v12.png");
        fs::write(&source, bytes).unwrap();
        let off = AtomicBool::new(false);
        for format in [
            Format::Png,
            Format::Jpeg,
            Format::Webp,
            Format::Avif,
            Format::Psd,
            Format::Tiff,
            Format::Jxl,
        ] {
            let converted = convert(&engine, &source, dir.path(), format, &off).unwrap();
            let restored = convert(&engine, &converted, dir.path(), Format::Png, &off).unwrap();
            assert!(fs::metadata(restored).unwrap().len() > 0);
        }
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let bad = dir.path().join("broken.png");
        fs::write(&bad, b"broken").unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        let error = convert(&engine, &bad, dir.path(), Format::Png, &off).unwrap_err();
        assert!(error.contains("identify"), "{error}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);

        // A tiny PNG with an invalid zlib stream but a valid IHDR and chunk CRCs.
        // Metadata inspection must succeed; only the actual conversion decodes IDAT.
        let corrupt = dir.path().join("invalid-idat.png");
        fs::write(
            &corrupt,
            include_bytes!("../tests/fixtures/invalid-idat-v121.png"),
        )
        .unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        let error = convert(&engine, &corrupt, dir.path(), Format::Png, &off).unwrap_err();
        assert!(error.contains("convert"), "{error}");
        assert!(!error.contains("identify"), "{error}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
        let error = convert(
            &engine,
            &dir.path().join("invalid-idat.png"),
            dir.path(),
            Format::Png,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error, "已取消");
    }
    #[test]
    fn heic_and_heif_are_rejected_before_engine_launch() {
        for ext in ["heic", "HEIF", "psb"] {
            assert!(Format::input(Path::new(&format!("image.{ext}"))).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn owned_scratch_cleanup_on_success_failure_cancel_and_timeout() {
        let peer = tempfile::Builder::new()
            .prefix("imb-c-job-")
            .tempdir()
            .unwrap();
        let sentinel = peer.path().join("magick-do-not-delete");
        fs::write(&sentinel, b"unrelated data").unwrap();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let parent_temp = std::env::var_os("TEMP");
        for mode in ["success", "failure", "cancel", "timeout"] {
            let input = tempfile::tempfile().unwrap();
            let mut output = tempfile::tempfile().unwrap();
            let cancel = AtomicBool::new(false);
            let action = match mode {
                "success" => "exit 0",
                "failure" => "exit 7",
                _ => "Start-Sleep -Seconds 30",
            };
            let script = format!(
                "$ErrorActionPreference='Stop'; if ($env:TEMP -ne $env:MAGICK_TEMPORARY_PATH -or $env:TMP -ne $env:TEMP) {{ throw 'temp mismatch' }}; [IO.File]::WriteAllText((Join-Path $env:TEMP 'magick-fixture'),'private image'); [Console]::Out.WriteLine($env:TEMP); [Console]::Out.Flush(); {action}"
            );
            let result = std::thread::scope(|scope| {
                if mode == "cancel" {
                    let observed = output.try_clone().unwrap();
                    let cancel = &cancel;
                    scope.spawn(move || {
                        let start = Instant::now();
                        while observed.metadata().unwrap().len() == 0
                            && start.elapsed() < Duration::from_secs(8)
                        {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        cancel.store(true, Ordering::Relaxed);
                    });
                }
                run_with_timeout(
                    &shell,
                    &["-NoProfile", "-NonInteractive", "-Command", &script],
                    &input,
                    &output,
                    &cancel,
                    if mode == "timeout" {
                        Duration::from_secs(3)
                    } else {
                        Duration::from_secs(10)
                    },
                )
            });
            output.seek(SeekFrom::Start(0)).unwrap();
            let mut recorded = String::new();
            output.read_to_string(&mut recorded).unwrap();
            let scratch = PathBuf::from(recorded.trim());
            assert!(
                scratch.is_absolute(),
                "child did not reach scratch fixture: {mode}: {result:?}"
            );
            assert!(
                !scratch.exists(),
                "scratch left after {mode}: {}",
                scratch.display()
            );
            assert_eq!(result.is_ok(), mode == "success", "{mode}: {result:?}");
            assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated data");
            assert_eq!(std::env::var_os("TEMP"), parent_temp);
        }
    }
    #[test]
    fn publish_never_overwrites_even_when_racing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("image.png"), b"original").unwrap();
        std::thread::scope(|s| {
            for _ in 0..8 {
                let p = dir.path();
                s.spawn(move || {
                    let mut t = NamedTempFile::new_in(p).unwrap();
                    t.write_all(b"new").unwrap();
                    publish(t, p, std::ffi::OsStr::new("image"), Format::Png).unwrap();
                });
            }
        });
        assert_eq!(fs::read(dir.path().join("image.png")).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 9);
    }
    #[test]
    fn cancelled_job_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            convert(
                Path::new("missing.exe"),
                Path::new("missing.png"),
                dir.path(),
                Format::Png,
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .contains("取消")
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn active_cancellation_terminates_child() {
        let cancel = AtomicBool::new(false);
        let input = tempfile::tempfile().unwrap();
        let output = tempfile::tempfile().unwrap();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(500));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(
                run(
                    &shell,
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "Start-Sleep -Seconds 30"
                    ],
                    &input,
                    &output,
                    &cancel
                )
                .unwrap_err()
                .contains("取消")
            );
        });
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    pub(super) fn inspect(engine: &Path) -> Command {
        let home = engine.parent().unwrap();
        let mut cmd = Command::new(engine);
        cmd.current_dir(home)
            .env("PATH", private_engine_path(home).unwrap())
            .env("MAGICK_HOME", home)
            .env("MAGICK_CONFIGURE_PATH", home)
            .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
            .env("MAGICK_FILTER_MODULE_PATH", home.join("modules/filters"))
            .env_remove("MAGICK_CONFIGURE_BUILD_PATH");
        cmd
    }
    fn pixel(engine: &Path, path: &Path) -> Vec<f64> {
        let r = inspect(engine)
            .args([
                "identify",
                "-format",
                "%[opaque] %[fx:p{0,0}.r] %[fx:p{0,0}.g] %[fx:p{0,0}.b] %[fx:p{0,0}.a]",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let text = String::from_utf8(r.stdout).unwrap();
        let mut fields = text.split_whitespace();
        let opaque = fields.next().unwrap().eq_ignore_ascii_case("true");
        let mut values: Vec<f64> = fields.map(|s| s.parse().unwrap()).collect();
        if opaque {
            values[3] = 1.0;
        }
        values
    }
    #[test]
    #[ignore = "Requires packaged v1.1 engine"]
    fn psd_composite_and_transparency() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let off = AtomicBool::new(false);
        let psd = dir.path().join("layered.psd");
        fs::write(&psd, include_bytes!("../tests/fixtures/layered.psd")).unwrap();
        let out = convert(&engine, &psd, dir.path(), Format::Png, &off).unwrap();
        let p = pixel(&engine, &out);
        assert!(
            p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01,
            "must use RED saved composite, not blue/green layers: {p:?}"
        );
        let no = dir.path().join("no-composite.psd");
        fs::write(&no, include_bytes!("../tests/fixtures/no-composite.psd")).unwrap();
        assert!(convert(&engine, &no, dir.path(), Format::Png, &off).is_err());
        let alpha = dir.path().join("alpha.png");
        fs::write(&alpha, include_bytes!("../tests/fixtures/alpha.png")).unwrap();
        for format in [
            Format::Png,
            Format::Webp,
            Format::Avif,
            Format::Psd,
            Format::Jpeg,
        ] {
            let file = convert(&engine, &alpha, dir.path(), format, &off).unwrap();
            let raster = convert(&engine, &file, dir.path(), Format::Png, &off).unwrap();
            let p = pixel(&engine, &raster);
            if matches!(format, Format::Jpeg) {
                assert!((p[3] - 1.0).abs() < 0.01, "JPEG must be opaque: {p:?}");
                assert!(
                    p[0] > 0.55 && p[1] > 0.7 && p[2] > 0.9,
                    "JPEG white background: {p:?}"
                );
            } else {
                assert!(
                    (p[3] - 128.0 / 255.0).abs() < 0.03,
                    "alpha lost: {format:?} {p:?}"
                );
                assert!(
                    (p[0] - 40.0 / 255.0).abs() < 0.08
                        && (p[1] - 130.0 / 255.0).abs() < 0.08
                        && (p[2] - 220.0 / 255.0).abs() < 0.08,
                    "color changed: {format:?} {p:?}"
                );
            }
        }
    }
    #[test]
    #[ignore = "Requires ImageMagick; IMBC_TEST_ENGINE selects the packaged engine"]
    fn imagemagick_matrix_and_failures() {
        let engine = std::env::var_os("IMBC_TEST_ENGINE")
            .map(PathBuf::from)
            .unwrap_or_else(locate_engine);
        assert!(engine.is_file());
        let dir = tempfile::tempdir().unwrap();
        let off = AtomicBool::new(false);
        let seed = dir.path().join("原圖 [1] %03d 空白.png");
        fs::write(&seed, include_bytes!("../tests/fixtures/seed.png")).unwrap();
        let original = fs::read(&seed).unwrap();
        let mut sources = vec![seed.clone()];
        for format in [Format::Webp, Format::Jpeg, Format::Avif, Format::Psd] {
            sources.push(convert(&engine, &seed, dir.path(), format, &off).unwrap());
        }
        for source in sources {
            let bytes = fs::read(&source).unwrap();
            for format in [
                Format::Webp,
                Format::Png,
                Format::Jpeg,
                Format::Avif,
                Format::Psd,
            ] {
                let output = convert(&engine, &source, dir.path(), format, &off).unwrap();
                assert_ne!(source, output);
                assert!(fs::metadata(&output).unwrap().len() > 0);
                let result = inspect(&engine)
                    .arg("identify")
                    .args(["-format", "%m %wx%h"])
                    .stdin(Stdio::from(File::open(&output).unwrap()))
                    .arg(format.input_coder())
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "inspect {format:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8_lossy(&result.stdout),
                    format!(
                        "{} 8x6",
                        match format {
                            Format::Webp => "WEBP",
                            Format::Png => "PNG",
                            Format::Jpeg => "JPEG",
                            Format::Avif => "AVIF",
                            Format::Psd => "PSD",
                            Format::Tiff => "TIFF",
                            Format::Jxl => "JXL",
                        }
                    )
                );
            }
            assert_eq!(fs::read(&source).unwrap(), bytes);
        }
        assert_eq!(fs::read(&seed).unwrap(), original);
        let bad = dir.path().join("bad.png");
        fs::write(&bad, b"broken").unwrap();
        let before = fs::read_dir(dir.path()).unwrap().count();
        assert!(convert(&engine, &bad, dir.path(), Format::Png, &off).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), before);
        let animation = dir.path().join("animation.webp");
        fs::write(
            &animation,
            include_bytes!("../tests/fixtures/animation.webp"),
        )
        .unwrap();
        assert!(
            convert(&engine, &animation, dir.path(), Format::Png, &off)
                .unwrap_err()
                .contains("靜態")
        );
        assert!(
            convert(
                Path::new("missing.exe"),
                &seed,
                dir.path(),
                Format::Png,
                &off
            )
            .is_err()
        );
        let apng = dir.path().join("animation.png");
        fs::write(&apng, b"\x89PNG\r\n\x1a\n\0\0\0\x08acTL\0\0\0\x02\0\0\0\0").unwrap();
        assert!(
            convert(&engine, &apng, dir.path(), Format::Png, &off)
                .unwrap_err()
                .contains("動畫")
        );
    }
}

#[cfg(test)]
#[path = "regression_r2.rs"]
mod regression_r2;

#[cfg(test)]
#[path = "regression_r3.rs"]
mod regression_r3;

#[cfg(test)]
#[path = "regression_final.rs"]
mod regression_final;

#[cfg(test)]
#[path = "regression_lowil_review.rs"]
mod regression_lowil_review;
#[cfg(test)]
#[path = "regression_security.rs"]
mod regression_security;
