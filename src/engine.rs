#[path = "color_diagnostics.rs"]
mod color_diagnostics;
#[path = "controlled_temp.rs"]
mod controlled_temp;
#[path = "destination.rs"]
mod destination;
#[path = "engine_limits.rs"]
mod engine_limits;
#[path = "engine_memory.rs"]
mod engine_memory;
#[path = "filesystem_safety.rs"]
mod filesystem_safety;
#[path = "instance_ledger.rs"]
mod instance_ledger;
#[path = "low_integrity.rs"]
mod low_integrity;
#[cfg(windows)]
#[path = "process_guard.rs"]
mod process_guard;
#[path = "runtime_cache.rs"]
mod runtime_cache;
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
use destination::{Destination, OutputTemp};
use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
type ProgressCallback = Box<dyn Fn(u8)>;
thread_local! { static PROGRESS: std::cell::RefCell<Option<ProgressCallback>> = const { std::cell::RefCell::new(None) }; }
struct ProgressGuard(Option<ProgressCallback>);
impl Drop for ProgressGuard {
    fn drop(&mut self) {
        PROGRESS.with(|p| *p.borrow_mut() = self.0.take());
    }
}
/// 診斷紀錄（RC2 第三輪 Release 驗收用）：只在設定 `IMBC_DIAG_LOG` 時，把各階段時間點、
/// 預留量與引擎呼叫耗時附加到該檔案。未設定時完全不寫入；不含圖片內容。
pub(crate) fn diag(event: &str, detail: impl std::fmt::Display) {
    static LOG: std::sync::OnceLock<Option<std::sync::Mutex<File>>> = std::sync::OnceLock::new();
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let log = LOG.get_or_init(|| {
        std::env::var_os("IMBC_DIAG_LOG").and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(std::sync::Mutex::new)
        })
    });
    if let Some(file) = log
        && let Ok(mut file) = file.lock()
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "{:.3}\t{:?}\t{event}\t{detail}",
            start.elapsed().as_secs_f64(),
            std::thread::current().id()
        );
    }
}
fn stage(value: u8) {
    PROGRESS.with(|p| {
        if let Some(callback) = p.borrow().as_ref() {
            callback(value);
        }
    });
}
/// 背景工作可訂閱讀取／中繼／編碼／驗證／發布階段（0–4）。
pub fn convert_with_progress(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    options: &Options,
    cancel: &AtomicBool,
    callback: impl Fn(u8) + 'static,
) -> Result<(PathBuf, Verification)> {
    let _guard = ProgressGuard(PROGRESS.with(|p| p.borrow_mut().replace(Box::new(callback))));
    convert_with_options(engine, input, dir, format, options, cancel)
}
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
    Psb,
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
            7 => Self::Psb,
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
            Self::Psb => "psb:-",
            Self::Tiff => "tiff:-",
            Self::Jxl => "jxl:-",
        }
    }
    fn input_coder(self) -> &'static str {
        // Read the saved PSD composite, never interpret layer frames as animation.
        if matches!(self, Self::Psd | Self::Psb) {
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
            Self::Psb => "psb",
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
            "psb" => Ok(Self::Psb),
            "tif" | "tiff" => Ok(Self::Tiff),
            "jxl" => Ok(Self::Jxl),
            _ => Err("僅支援 WebP、PNG、JPEG、AVIF、PSD、PSB、TIFF、JXL；不支援 HEIC/HEIF".into()),
        }
    }
}

/// GUI／CLI 以靜態 CRT 建置，不需要應用程式目錄內的任何 DLL：動態 DLL 搜尋只允許 System32，
/// 排除應用程式目錄、目前目錄與 PATH（原生引擎是另一個程序，由 runtime 快取與 AppContainer 控制）。
pub fn initialize_dll_search() -> std::io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetDefaultDllDirectories(flags: u32) -> i32;
    }
    // LOAD_LIBRARY_SEARCH_SYSTEM32
    if unsafe { SetDefaultDllDirectories(0x800) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn locate_engine() -> PathBuf {
    // Development and release both use this copy's private engine.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("ImageMagick").join("magick.exe")))
        .unwrap_or_default()
}

pub fn external_validation_memory_mib() -> u64 {
    engine_memory::external_ceiling_mib()
}
pub struct ExternalProcess {
    child: low_integrity::Child,
    job: process_guard::ProcessGuard,
}
impl ExternalProcess {
    #[cfg(test)]
    pub fn process_handles(&self) -> std::io::Result<Vec<std::os::windows::io::OwnedHandle>> {
        self.job.process_handles()
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn stop(&self) -> std::io::Result<()> {
        self.job.stop_and_wait()
    }
}
impl Drop for ExternalProcess {
    fn drop(&mut self) {
        let _ = self.job.stop_and_wait();
    }
}
#[cfg(test)]
pub fn spawn_external_command(
    exe: &Path,
    args: &[std::ffi::OsString],
    stdout: &File,
    stderr: &File,
) -> std::io::Result<ExternalProcess> {
    spawn_external_budgeted(exe, args, stdout, stderr, 1216)
}
pub fn spawn_external_budgeted(
    exe: &Path,
    args: &[std::ffi::OsString],
    stdout: &File,
    stderr: &File,
    memory_mib: u64,
) -> std::io::Result<ExternalProcess> {
    let job = process_guard::ProcessGuard::external_with_memory(memory_mib as usize * 1024 * 1024)?;
    let child = low_integrity::spawn_regular(exe, args, stdout, stderr, job.handle())?;
    Ok(ExternalProcess { child, job })
}
pub struct ValidationTemp(controlled_temp::ControlledDir);
impl ValidationTemp {
    pub fn new() -> std::io::Result<Self> {
        controlled_temp::ControlledDir::external(&controlled_temp::preferred_root()?).map(Self)
    }
    pub fn path(&self) -> &Path {
        self.0.path()
    }
    pub fn retain(&self) {
        self.0.retain();
    }
}
pub struct ValidationMemory {
    _lease: engine_memory::Lease,
}
pub fn reserve_validation_memory(mib: u64, cancel: &AtomicBool) -> Result<ValidationMemory> {
    let start = Instant::now();
    Ok(ValidationMemory {
        _lease: engine_memory::Lease::external(mib, || {
            if cancel.load(Ordering::Relaxed) {
                return Err("未完成（已取消）".into());
            }
            if start.elapsed() > Duration::from_secs(120) {
                return Err("外部驗證等待資源逾時".into());
            }
            Ok(())
        })?,
    })
}

/// 啟動時回收已終止 IMB-C 執行個體的殘留（P1-C）：只處理其擁有權日誌中 File ID 相符、
/// 未被使用的項目，並終止其建立→納管空窗留下的暫停子程序。沒有需要回報的事項時回傳空集合。
#[allow(dead_code)] // GUI 使用；驗收工具共用 engine.rs
pub fn recover_stale_temp() -> Vec<String> {
    // 建立 AppContainer 與恢復子程序之間的極短空窗若崩潰，會留下 profile；
    // 只回收建立者程序已結束的 IMB-C.Engine.* 項目。不影響執行中的其他執行個體。
    let profiles = low_integrity::recover_stale_profiles();
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
    if !profiles.is_empty() {
        lines.push(format!(
            "已回收先前異常結束留下的引擎隔離設定 {} 項",
            profiles.len()
        ));
    }
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
    // IMB-C 為本工作產生、須以路徑交給引擎讀取的檔案（目標 ICC 快照）；每次呼叫只對
    // 該次 AppContainer SID 授權唯讀，工作目錄本身仍不開放。
    static JOB_ENGINE_READABLE: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
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
        JOB_ENGINE_READABLE.with(|v| v.borrow_mut().clear());
        JOB_IMAGE.with(|v| v.set((0, 16, 4)));
        // Low IL 引擎無法存取 UNC／網路磁碟機；暫存改放本機 NTFS 並明確告知實際路徑。
        // 圖片輸入與輸出仍可在 UNC，由 Rust 控制代碼讀寫。
        let (root, redirected) = controlled_temp::local_engine_root(requested).map_err(err)?;
        let root = root.as_path();
        if let Some(original) = redirected {
            WARNINGS.with(|w| {
                w.borrow_mut().push(format!(
                    "暫存根目錄 {} 是網路路徑，隔離引擎（AppContainer）無法存取；本次改用本機受控暫存 {}",
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
            // 取消必須維持精確的「已取消」（M1）：一般提示對已取消的工作沒有意義，不附加。
            // 暫存保留、清理失敗等異常由 finish 併入錯誤本身，仍會顯示。
            Err(e) => Err(if warnings.is_empty() || e == "已取消" {
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
        #[cfg(test)]
        JOB_BUDGET.with(|b| {
            if let Some(budget) = b.borrow().as_ref() {
                TEST_LAST_BUDGET.with(|v| v.set(budget.usage()));
            }
        });
        JOB_BUDGET.with(|b| *b.borrow_mut() = None);
        JOB_RESERVATIONS.with(|r| r.borrow_mut().clear());
        JOB_ENGINE_READABLE.with(|v| v.borrow_mut().clear());
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
    if policy != include_bytes!("../assets/policy-v14.xml") {
        return Err("私有 policy 與 v1.4 預期內容不符，請校驗 runtime".into());
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
    // 生效 policy 應來自實際載入的 runtime 快取副本。
    let engine = &cached_engine(engine)?;
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
    // 每次呼叫獨立 AppContainer；建立失敗即拒絕，不退回只有 Low IL 的模式。
    let sandbox = low_integrity::Sandbox::new()
        .map_err(|e| format!("無法建立引擎隔離（AppContainer），未讀取圖片：{e}"))?;
    scratch
        .allow_sandbox(&sandbox)
        .map_err(|e| format!("無法設定引擎隔離工作目錄：{e}"))?;
    for path in JOB_ENGINE_READABLE.with(|v| v.borrow().clone()) {
        match sandbox.grant_file_read(&path) {
            Ok(()) => {}
            // 快照已在前一階段刪除：沒有可授權的對象。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("無法授權引擎讀取 {}：{e}", display_path(&path))),
        }
    }
    let native = scratch.child("imb-c-cache-").map_err(err)?;
    native
        .allow_sandbox(&sandbox)
        .map_err(|e| format!("無法設定引擎隔離快取目錄：{e}"))?;
    controlled_temp::check_free(&scratch_path, 2 * 1024 * 1024 * 1024).map_err(err)?;
    let mut invocation = Invocation {
        scratch: Some(scratch),
        stopped: true,
    };
    let invoked = Instant::now();
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
    diag(
        "engine",
        format_args!(
            "{} {:.3}s ok={} args={}",
            engine.file_name().unwrap_or_default().to_string_lossy(),
            invoked.elapsed().as_secs_f64(),
            result.is_ok(),
            args.iter()
                .filter(|a| a.ends_with(":-") || a.starts_with('-') && a.len() < 3)
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        ),
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

/// 引擎一律從已驗證、已鎖定的 runtime 快取啟動（不修改 Portable 目錄 ACL）。
fn cached_engine(engine: &Path) -> Result<PathBuf> {
    runtime_cache::resolve(engine)
        .map_err(|e| format!("無法準備引擎 runtime 快取，未讀取圖片：{e}"))
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
    let engine = &cached_engine(engine)?;
    #[cfg(test)]
    TEST_ENGINE_INPUTS.with(|v| {
        v.borrow_mut().push(
            filesystem_safety::final_path(source)
                .unwrap_or_else(|e| PathBuf::from(format!("ERR {e}"))),
        )
    });
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
    let guard = process_guard::ProcessGuard::engine(memory.0)
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
            loop {
                let resource_error = (|| {
                    // native cache 仍要求只含一般檔案；用量以引擎唯一可寫的整棵暫存樹計算。
                    controlled_temp::native_size(scratch).map_err(err)?;
                    let size = controlled_temp::tree_size(profile).map_err(err)?;
                    if size > engine_limits::scratch_limit() {
                        return Err(format!(
                            "引擎暫存觀察用量超過 {} MiB，未保存輸出",
                            engine_limits::scratch_limit() / (1024 * 1024)
                        ));
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
                    // 每次輪詢（約 40 ms）都檢查暫存磁碟區剩餘空間，縮小超量寫入窗口。
                    controlled_temp::check_free(scratch, 2 * 1024 * 1024 * 1024).map_err(err)?;
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
            .take(engine_limits::STDERR_LIMIT)
            .read_to_end(&mut detail)
            .map_err(|secondary| format!("{e}；stderr 診斷讀取失敗：{secondary}"))?;
        let detail = diagnostic_text(&String::from_utf8_lossy(&detail));
        return Err(if detail.trim().is_empty() {
            e
        } else {
            format!("{e}：{}", detail.chars().take(4096).collect::<String>())
        });
    }
    Ok(())
}

// 先記錄程序樹確認結果，再收回兩個執行緒；任何一個 panic 都不能略過另一個 join。
fn diagnostic_text(stderr: &str) -> String {
    const START: &str =
        "IMB-C distribution: linked LGPL libraries (not a Microsoft Runtime license):";
    const END: &str = "Full authors, corresponding source and replacement instructions: ../THIRD-PARTY-RUNTIME.txt and ../LGPL-REPLACEMENT.md.";
    let lines: Vec<_> = stderr.lines().collect();
    if lines.first().copied() == Some(START)
        && let Some(end) = lines.iter().take(20).position(|l| *l == END)
    {
        let legal_prefixes = [
            "GLib ",
            "FriBidi ",
            "GLib/FriBidi:",
            "liblqr ",
            "GPL v3:",
            "libheif ",
            "License/source/build and replacement:",
            "libaom ",
        ];
        if lines[1..end]
            .iter()
            .all(|line| legal_prefixes.iter().any(|p| line.starts_with(p)))
        {
            return lines[end + 1..].join("\n").trim().to_owned();
        }
    }
    stderr.trim().to_owned()
}

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
        #[cfg(test)]
        TEST_COPY_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(n as u64);
            }
        });
    }
    Ok(())
}
// 測試：每寫完一個複製區塊後呼叫（模擬已寫入部分資料後取消、中斷）。
#[cfg(test)]
type CopyHook = Option<Box<dyn FnMut(u64)>>;
#[cfg(test)]
thread_local! { static TEST_COPY_HOOK: std::cell::RefCell<CopyHook> = const { std::cell::RefCell::new(None) }; }

/// 以「只共享讀取」開啟來源：轉檔期間其他程序無法以寫入權限開啟，避免同長度的並行修改
/// 讓引擎讀到與複製／驗證不同的內容。來源正被寫入時明確拒絕。
fn open_source(input: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(input)
        .map_err(|e| {
            if e.raw_os_error() == Some(32) {
                "來源正被其他程式寫入或鎖定，未轉檔；請關閉該程式後再試".to_owned()
            } else {
                format!("開啟來源失敗：{e}")
            }
        })
}
/// 複製失敗的錯誤文字；取消必須保持精確的「已取消」，GUI／CLI／JSON 才能正確分類（M1）。
fn copy_failure(context: &str, e: String) -> String {
    if e == "已取消" {
        e
    } else {
        format!("{context}：{e}")
    }
}
/// 來源不在本機 NTFS（FAT／exFAT、ReFS、SMB 等）時，從已開啟的控制代碼複製到
/// 本工作的受控暫存（本機 NTFS，DACL 未授權引擎 AppContainer），引擎只拿到副本的
/// 唯讀控制代碼。遠端與能力都由同一控制代碼判定；只讀原控制代碼、不以路徑重新開啟。
/// 複製期間來源長度改變即拒絕。無法判定本機能力時由 `check` 拒絕。
fn isolated_source(mut source: File, cancel: &AtomicBool) -> Result<File> {
    match filesystem_safety::check(&source).map_err(err)? {
        filesystem_safety::Protection::LocalNtfs => Ok(source),
        filesystem_safety::Protection::CopyFirst(filesystem) => {
            let len = source.metadata().map_err(err)?.len();
            // 本機暫存已由呼叫端在複製與 identify 前預留（見 convert_with_options_inner）。
            let mut copy = job_file()?;
            source.seek(SeekFrom::Start(0)).map_err(err)?;
            copy_bytes(&mut source, &mut copy, len, cancel)
                .map_err(|e| copy_failure(&format!("來源（{filesystem}）複製到受控暫存失敗"), e))?;
            let mut extra = [0u8; 1];
            if source.read(&mut extra).map_err(err)? != 0
                || source.metadata().map_err(err)?.len() != len
            {
                return Err("來源在複製到受控暫存期間改變，未轉檔".into());
            }
            copy.seek(SeekFrom::Start(0)).map_err(err)?;
            #[cfg(test)]
            TEST_SOURCE_COPIES.with(|v| v.set(v.get() + 1));
            Ok(copy)
        }
    }
}
#[cfg(test)]
thread_local! { static TEST_LAST_BUDGET: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) }; }
#[cfg(test)]
thread_local! { static TEST_SOURCE_COPIES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
// 每次引擎呼叫的 stdin 控制代碼實際路徑（H3：引擎拿到的檔案只能是受控暫存或本機 NTFS 來源）。
#[cfg(test)]
thread_local! { static TEST_ENGINE_INPUTS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) }; }

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

/// 目的地 `.tmp*` 的擁有權日誌項目（P1-C）。宣告於 OutputTemp 之前，使其較晚 drop：
/// 檔案先刪除或發布，項目才釋放。崩潰時留下的項目由下次啟動以 File ID 核對後回收。
/// 檔案系統不提供可靠 File ID（部分 SMB／NAS）時不記錄：只會漏收，不會誤刪。
struct TempRecord(Option<instance_ledger::Entry>);
impl TempRecord {
    fn none() -> Self {
        Self(None)
    }
    fn record(temp: &OutputTemp) -> Result<Self> {
        let ledger = instance_ledger::global().map_err(err)?;
        Ok(Self(
            ledger
                .record(instance_ledger::Kind::File, temp.as_file(), temp.path())
                .ok(),
        ))
    }
    /// 只有確定已發布或刪除，才從日誌釋放項目。其他退出路徑保留證據。
    fn release(self) {
        if let Some(entry) = self.0 {
            entry.release();
        }
    }
}

/// 以同一目的目錄控制代碼不覆寫發布；同名已存在時改用「名稱 (n)」。
fn publish(
    temp: &mut OutputTemp,
    destination: &Destination,
    stem: &std::ffi::OsStr,
    format: Format,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    temp.as_file().sync_all().map_err(err)?;
    for index in 0..10000 {
        // 每次嘗試發布前都檢查真實取消旗標與期限（第三輪：原本固定為 false）。
        check_job(cancel)?;
        let mut name = OsString::from(stem);
        if index > 0 {
            name.push(format!(" ({index})"));
        }
        name.push(format!(".{}", format.extension()));
        match temp.persist_noclobber(destination, &name) {
            Ok(target) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(err(e)),
        }
    }
    Err("同名檔案過多，無法產生唯一名稱".into())
}

/// 將已在本機受控暫存完成驗證的輸出交付到目的端（H3／M2）：
/// 引擎從未取得目的端控制代碼；IMB-C 以目的目錄控制代碼相對建立暫存、寫入、讀回比對
/// SHA-256，確認與已驗證的位元組一致後才不覆寫發布。任何失敗都以控制代碼刪除暫存，
/// 刪除失敗（SMB 中斷等）會回報並保留擁有權紀錄（File ID）給下次啟動安全回收。
fn deliver(
    local: &mut File,
    destination: &Destination,
    stem: &std::ffi::OsStr,
    format: Format,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let length = local.metadata().map_err(err)?.len();
    if length == 0 {
        return Err("輸出為空".into());
    }
    let free = destination
        .free_bytes()
        .map_err(|e| format!("無法確認輸出磁碟剩餘空間：{e}"))?;
    let needed = length.saturating_add(256 * 1024 * 1024);
    if free < needed {
        return Err(format!(
            "輸出磁碟空間不足：{} 可用 {} MiB，寫入輸出需要約 {} MiB（含安全餘裕；不是硬配額），未保存輸出",
            display_path(destination.path()),
            free / 1048576,
            needed / 1048576
        ));
    }
    let verified = sha256_cancellable(local, cancel)?;
    diag(
        "deliver_hashed",
        format_args!("bytes={length} destination_free={free}"),
    );
    let mut record = TempRecord::none();
    let mut temp = destination
        .create_temp()
        .map_err(|e| format!("無法在輸出目錄建立暫存：{e}"))?;
    let staged = (|| {
        record = TempRecord::record(&temp)?;
        local.seek(SeekFrom::Start(0)).map_err(err)?;
        copy_bytes(local, temp.as_file_mut(), length, cancel)
            .map_err(|e| copy_failure("寫入輸出暫存失敗", e))?;
        temp.as_file().sync_all().map_err(err)?;
        if sha256_cancellable(temp.as_file_mut(), cancel)? != verified {
            return Err("輸出暫存與已驗證內容不一致，未保存輸出".into());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        diag("deliver_written", "");
        publish(&mut temp, destination, stem, format, cancel)
    })();
    diag("deliver_done", format_args!("ok={}", staged.is_ok()));
    match staged {
        Ok(path) => {
            record.release();
            Ok(path)
        }
        Err(error) => {
            let location = display_path(temp.path());
            match temp.discard() {
                Ok(()) => {
                    record.release();
                    Err(error)
                }
                Err(e) => {
                    // 不釋放擁有權紀錄：下次啟動以 File ID 核對後回收，不依名稱刪除。
                    let tracked = record.0.is_some();
                    Err(format!(
                        "{error}；輸出暫存 {location} 無法刪除（{e}），未發布；{}",
                        if tracked {
                            "已保留其擁有權紀錄，下次啟動時核對檔案身分後回收"
                        } else {
                            "此位置未提供可靠的檔案身分，無法自動回收，請確認後手動刪除"
                        }
                    ))
                }
            }
        }
    }
}
/// 本機暫存的預先預留（例如 identify 前就要寫入的來源副本）；工作結束時釋放。
fn reserve_job_temp(bytes: u64) -> Result<()> {
    let reservation = controlled_temp::reserve_bytes(&job_dir(), bytes)
        .map_err(|e| format!("磁碟資源預檢失敗：{e}"))?;
    JOB_RESERVATIONS.with(|r| r.borrow_mut().push(reservation));
    Ok(())
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
    pub lossy_quality: u8,
    pub avif_quality: u8,
    pub avif_lossless: bool,
    pub jxl_quality: u8,
    pub jxl_lossless: bool,
    pub webp_lossless: bool,
    pub target_icc: Option<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            lossy_quality: 90,
            avif_quality: 90,
            avif_lossless: false,
            jxl_quality: 90,
            jxl_lossless: true,
            webp_lossless: true,
            target_icc: None,
        }
    }
}

#[derive(Debug)]
pub struct Verification {
    pub lossless_requested: bool,
    pub composite_decoded: bool,
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
            (if self.lossless_requested
                && !self.transformed
                && !self.composite_decoded
                && self.input_depth == self.output_depth
            {
                "lossless mode"
            } else {
                "lossy / composite mode"
            })
            .to_owned(),
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
            (if self.composite_decoded {
                "compared after composite Alpha decoding; original composite samples not verified"
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

// ICC v4 cicp tag provides explicit primaries/transfer. Arbitrary ICC curves
// cannot be mapped to CICP without color science; keep them unspecified instead.
fn icc_cicp(profile: &mut File) -> Result<(u8, u8)> {
    let len = profile.metadata().map_err(err)?.len();
    profile.seek(SeekFrom::Start(128)).map_err(err)?;
    let mut count = [0u8; 4];
    profile.read_exact(&mut count).map_err(err)?;
    let count = u64::from(u32::from_be_bytes(count));
    if count > 65536 || 132 + count * 12 > len {
        return Err("ICC 標籤表長度無效".into());
    }
    for _ in 0..count {
        let mut tag = [0u8; 12];
        profile.read_exact(&mut tag).map_err(err)?;
        if &tag[..4] == b"cicp" {
            let offset = u64::from(u32::from_be_bytes(tag[4..8].try_into().unwrap()));
            let size = u64::from(u32::from_be_bytes(tag[8..].try_into().unwrap()));
            if size < 12 || offset < 132 + count * 12 || offset + size > len {
                return Err("ICC CICP 標籤長度無效".into());
            }
            profile.seek(SeekFrom::Start(offset)).map_err(err)?;
            let mut data = [0u8; 12];
            profile.read_exact(&mut data).map_err(err)?;
            if &data[..4] != b"cicp" {
                return Err("ICC CICP 標籤型別無效".into());
            }
            return Ok((data[8], data[9]));
        }
    }
    Ok((2, 2))
}

fn png_lossless_primaries_transfer(source: &mut File) -> Result<(u8, u8)> {
    // png_profile_hint already validated the complete chunk boundaries.
    source.seek(SeekFrom::Start(8)).map_err(err)?;
    let mut explicit_color = false;
    let mut srgb = false;
    loop {
        let mut header = [0u8; 8];
        source.read_exact(&mut header).map_err(err)?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap());
        match &header[4..] {
            b"sRGB" => srgb = true,
            b"cHRM" | b"gAMA" => explicit_color = true,
            b"IEND" => break,
            _ => {}
        }
        source
            .seek(SeekFrom::Current(i64::from(size) + 4))
            .map_err(err)?;
    }
    if srgb {
        Ok((1, 13))
    } else if explicit_color {
        Err("AVIF 真無損無法確認此 PNG 的 cHRM／gAMA 對應色彩資訊；請使用嵌入正確 ICC 的來源或 PNG／TIFF／JXL".into())
    } else {
        Ok((2, 2))
    }
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

fn verify_alpha_samples(a: &mut File, b: &mut File, depth: u32, cancel: &AtomicBool) -> Result<()> {
    a.seek(SeekFrom::Start(0)).map_err(err)?;
    b.seek(SeekFrom::Start(0)).map_err(err)?;
    let stride = if depth == 32 { 16 } else { 8 };
    let sample = stride / 4;
    let mut aa = [0u8; 65536];
    let mut bb = [0u8; 65536];
    loop {
        check_job(cancel)?;
        let n = a.read(&mut aa).map_err(err)?;
        if n == 0 {
            return Ok(());
        }
        b.read_exact(&mut bb[..n]).map_err(err)?;
        if n % stride != 0 {
            return Err("Alpha 驗證資料長度無效".into());
        }
        for (x, y) in aa[..n]
            .chunks_exact(stride)
            .zip(bb[..n].chunks_exact(stride))
        {
            if x[stride - sample..] != y[stride - sample..] {
                return Err("JXL 有損輸出的 Alpha 改變，未保存輸出".into());
            }
        }
    }
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
    stage(0);
    let input_format = Format::input(input)?;
    let source = open_source(input)?;
    // 目的目錄只以路徑開啟一次並固定；之後的能力、空間、暫存與發布都用同一控制代碼。
    // 輸出由 IMB-C 寫入；無法判定能力的磁碟區拒絕。
    let destination = Destination::open(dir).map_err(err)?;
    if !source.metadata().map_err(err)?.is_file() {
        return Err("來源不是一般檔案".into());
    }
    // H1：寫入任何資料前先跨執行個體預留本機暫存——來源副本（非本機 NTFS 時）與 identify
    // 階段引擎可用的暫存上限。完整預留（含像素估算）的最低需求本來就包含這兩項，
    // 因此不會提高合法轉檔的門檻。
    reserve_job_temp(
        engine_limits::scratch_limit().saturating_add(source.metadata().map_err(err)?.len()),
    )?;
    diag("job_start", display_path(input));
    let mut source = isolated_source(source, cancel)?;
    diag("source_ready", "");
    verify_active_policy(engine, cancel)?;
    diag("policy_verified", "");
    if matches!(input_format, Format::Psd | Format::Psb) {
        let mut header = [0; 6];
        source
            .read_exact(&mut header)
            .map_err(|e| format!("PSD 檔頭截斷：{e}"))?;
        if &header[..4] != b"8BPS" || !matches!(&header[4..], [0, 1] | [0, 2]) {
            return Err("僅支援 PSD v1／PSB v2；Photoshop 檔頭無效或版本不支援".into());
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
    diag(
        "identified",
        format_args!("{width}x{height} depth={depth} channels={channels}"),
    );
    if width.saturating_mul(height) > 400_000_000 {
        return Err("圖片超過 400 MP（4 億像素）上限".into());
    }
    // 把 identify 前的早期預留在同一把跨程序鎖內原子升級為完整預留：
    // 不出現未預留空窗，也不把同一工作的兩筆額度重複計入（RC2 第三輪）。
    let early = JOB_RESERVATIONS.with(|r| r.borrow_mut().pop());
    let reservations = controlled_temp::preflight(
        &job_dir(),
        destination.path(),
        width.saturating_mul(height),
        source.metadata().map_err(err)?.len(),
        early.as_ref(),
    );
    // 升級失敗時早期預留維持原狀，仍由本工作持有到結束。
    JOB_RESERVATIONS.with(|r| r.borrow_mut().extend(early));
    let reservations = reservations.map_err(|e| format!("磁碟資源預檢失敗：{e}"))?;
    JOB_RESERVATIONS.with(|r| r.borrow_mut().extend(reservations));
    diag("reserved_full", "");
    // 2026-11-18 曾嘗試把累計寫入上限降到本工作的磁碟預留量，但累計量包含已刪除的中介檔，
    // 實測使合法的 100 MP AVIF 轉檔失敗，因此維持固定 32 GiB 上限（見 v1.4-rc2-appcontainer.md）。
    if matches!(format, Format::Psd) && (width > 30000 || height > 30000) {
        return Err("PSD 尺寸上限為每邊 30000 像素；請明確選擇 PSB，仍受既有資源上限限制".into());
    }
    if !(1..=100).contains(&options.avif_quality) {
        return Err("AVIF 品質須為 1–100".into());
    }
    if matches!(format, Format::Avif)
        && options.avif_lossless
        && fields[4].to_ascii_lowercase().starts_with("cmyk")
    {
        return Err("AVIF 真無損不支援 CMYK；請保留原檔並選擇能保留原樣本與 ICC 的格式，或明確選擇色彩轉換及有損模式".into());
    }
    if matches!(format, Format::Avif)
        && options.avif_lossless
        && fields[4].to_ascii_lowercase().starts_with("gray")
    {
        return Err("AVIF 真無損目前僅支援 8-bit RGB／RGBA；此灰階路徑的編碼器不支援 identity matrix，請選 PNG／TIFF／JXL".into());
    }
    if !(1..=100).contains(&options.jxl_quality) {
        return Err("JXL 品質須為 1–100".into());
    }
    if !(1..=100).contains(&options.lossy_quality) {
        return Err("有損品質須為 1–100".into());
    }
    if !options.jxl_lossless && options.jxl_quality == 100 && matches!(format, Format::Jxl) {
        return Err("JXL 有損品質須為 1–99；品質 100 請明確選擇無損模式".into());
    }
    // PSD/PSB composite pixels are matted against white by the writer.
    // Even an opaque Alpha channel changes the declared Alpha semantics.
    let mut source_alpha = fields[4].to_ascii_lowercase().ends_with('a');
    let (input_depth, source_icc, _, _) =
        metadata(engine, input_format.input_coder(), &mut source, cancel)?;
    if matches!(input_format, Format::Tiff) && matches!(format, Format::Psd | Format::Psb) {
        // TIFF 的 ping 可能未建立 Alpha trait，不能據此放行 Photoshop 輸出。
        source.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut channels = job_file()?;
        run(
            engine,
            &[
                "identify",
                "-format",
                "%[channels]",
                input_format.input_coder(),
            ],
            &source,
            &channels,
            cancel,
        )?;
        channels.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut text = String::new();
        channels.take(256).read_to_string(&mut text).map_err(err)?;
        let model = text
            .split_whitespace()
            .next()
            .ok_or("無法確認 TIFF Alpha 通道")?;
        source_alpha = model.to_ascii_lowercase().ends_with('a');
    }
    if matches!(input_format, Format::Psd | Format::Psb) && source_alpha {
        WARNINGS.with(|w|w.borrow_mut().push("PSD／PSB 合成 Alpha 經解碼器反混合；比對解碼後樣本，不代表原始合成影像的樣本與 Alpha 語意無損".into()));
    }
    if matches!(format, Format::Psd | Format::Psb) && source_alpha {
        return Err(
            "PSD／PSB 合成影像的半透明 RGB 會與白色混合，不能無損保留 Alpha；請選 PNG／TIFF／JXL"
                .into(),
        );
    }
    let mut source_profile_hash = None;
    let mut effective_icc_model = None;
    let mut source_cicp = (2, 2);
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
        if options.avif_lossless && matches!(format, Format::Avif) {
            source_cicp = icc_cicp(&mut profile)?;
        }
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
    let mut lossless_pt = source_cicp;
    if !source_icc && options.avif_lossless && matches!(format, Format::Avif) {
        lossless_pt = match input_format {
            Format::Png => png_lossless_primaries_transfer(&mut source)?,
            Format::Webp => (1, 13),
            _ => (2, 2),
        };
    }
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
        if text.trim().is_empty() || text.trim() == "2/2/0/1" {
            color_missing = true;
        } else {
            validate_avif_cicp(&text)?;
            let transfer = text.trim().split('/').nth(1).ok_or("CICP transfer 缺漏")?;
            lossless_pt = (1, transfer.parse().map_err(err)?);
            avif_output_cicp = format!("heic:cicp=1/{transfer}/6/1");
        }
    }
    if options.webp_lossless && matches!(format, Format::Webp) && input_depth > 8 {
        return Err("WebP 僅支援 8-bit；原圖高於 8-bit，無法真正無損。請選 PNG/TIFF/JXL，或關閉 WebP 無損以允許降低位深".into());
    }
    if input_depth > 16 && !matches!(format, Format::Jxl | Format::Tiff) {
        return Err("超過 16-bit／浮點來源請使用 JXL 或 TIFF，避免靜默截斷精度".into());
    }
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
        JOB_ENGINE_READABLE.with(|v| v.borrow_mut().push(profile.path().to_owned()));
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
    if matches!(format, Format::Psd | Format::Psb) {
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
    stage(1);
    diag("intermediate_start", "");
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
    // 輸出先寫入本機受控暫存；驗證完成前引擎與目的端互不接觸（H3）。
    let mut output = job_file()?;
    // ICC conversion can produce fractional samples from 8-bit input.
    // Quantize only to 16-bit for formats that support it, matching verification.
    let depth = if input_depth > 16 {
        "32"
    } else if input_depth > 8
        || (options.target_icc.is_some()
            && matches!(
                format,
                Format::Png | Format::Tiff | Format::Jxl | Format::Psd | Format::Psb
            ))
        || (matches!(input_format, Format::Psd | Format::Psb)
            && source_alpha
            && matches!(format, Format::Png | Format::Tiff | Format::Jxl))
    {
        "16"
    } else {
        "8"
    };
    let q = if options.avif_lossless {
        100
    } else {
        options.avif_quality
    }
    .to_string();
    let jxl_q = options.jxl_quality.to_string();
    let lossy_q = options.lossy_quality.to_string();
    if options.avif_lossless && matches!(format, Format::Avif) {
        if input_depth > 8 || options.target_icc.is_some() {
            return Err(
                "AVIF 真無損目前只接受 8-bit 且未套用 ICC 轉換的來源；高位深請使用 PNG／TIFF／JXL"
                    .into(),
            );
        }
        // ICC remains authoritative. Unspecified P/T avoids falsely relabeling
        // arbitrary ICC profiles as sRGB; matrix/range are always explicit.
        avif_output_cicp = format!("heic:cicp={}/{}/0/1", lossless_pt.0, lossless_pt.1);
        if !source_icc && lossless_pt == (2, 2) {
            color_missing = true;
            WARNINGS.with(|w|w.borrow_mut().push("來源未提供可確認的色彩描述；AVIF 保持未指定 primaries／transfer，不將未知色彩標成 sRGB".into()));
        }
    }
    let mut args = vec![
        "-limit", "thread", "1", "-limit", "memory", "256MiB", "-limit", "map", "512MiB", "-limit",
        "disk", "8GiB", "miff:-", "-depth", depth,
    ];
    if input_depth > 16 {
        args.extend(["-define", "quantum:format=floating-point"]);
    }
    if matches!(format, Format::Jpeg) {
        args.extend(["-quality", &lossy_q]);
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
            args.extend(["-quality", &lossy_q, "-define", "webp:lossless=false"]);
        }
    }
    if matches!(format, Format::Avif) {
        // The bundled encoder otherwise omits nclx for ordinary unprofiled SDR.
        // Record its existing sRGB/BT.601 full-range encoding explicitly, so our
        // own output also satisfies the strict CICP-only input validation.
        if options.avif_lossless || (!source_icc && !color_missing) {
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
        if options.jxl_lossless {
            args.extend(["-define", "jxl:distance=0", "-quality", "100"]);
        } else {
            args.extend(["-quality", &jxl_q]);
        }
    }
    if matches!(format, Format::Psd | Format::Psb) {
        args.extend(["-define", "psd:write-layers=true", "-compress", "RLE"]);
    }
    args.push(format.coder());
    stage(2);
    diag("encode_start", "");
    run(engine, &args, &intermediate, &output, cancel)
        .map_err(|e| image_stage_error("圖片轉換 (convert)", e))?;
    if output.metadata().map_err(err)?.len() == 0 {
        return Err("轉換輸出為空".into());
    }
    stage(3);
    diag(
        "verify_start",
        format_args!("output_bytes={}", output.metadata().map_or(0, |m| m.len())),
    );
    let (output_depth, output_icc, output_width, output_height) =
        metadata(engine, format.input_coder(), &mut output, cancel)?;
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
        let mut b = extracted(engine, format.input_coder(), &mut output, "icc", 16, cancel)?;
        if Some(sha256(&mut b)?) != expected_profile_hash {
            return Err(if matches!(format, Format::Jxl) && !options.jxl_lossless {
                "JXL 有損編碼器無法原樣保留此 ICC，未保存輸出；請改用 JXL 無損或 PNG／TIFF".into()
            } else {
                "ICC 描述檔不一致，未保存輸出".into()
            });
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
        &mut output,
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
    if matches!(format, Format::Jxl) && !options.jxl_lossless {
        verify_alpha_samples(&mut a, &mut b, check_depth, cancel)?;
        if output_depth != depth.parse::<u32>().map_err(err)? {
            return Err("JXL 有損輸出位深不符，未保存輸出".into());
        }
    }
    let lossless = matches!(
        format,
        Format::Png | Format::Tiff | Format::Psd | Format::Psb
    ) || (matches!(format, Format::Jxl) && options.jxl_lossless)
        || (matches!(format, Format::Avif) && options.avif_lossless)
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
    stage(4);
    diag("deliver_start", "");
    let result = deliver(
        &mut output,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        format,
        cancel,
    )?;
    Ok((
        result,
        Verification {
            lossless_requested: lossless,
            composite_decoded: matches!(input_format, Format::Psd | Format::Psb) && source_alpha,
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
                vec!["來源缺少可確認的 ICC／CICP；沿用解碼器行為完成轉檔，僅驗證樣本數值，未驗證色域／傳輸特性；請保留原檔".into()]
            } else {
                Vec::new()
            },
        },
    ))
}

pub fn sha256(file: &mut File) -> Result<String> {
    sha256_cancellable(file, &AtomicBool::new(false))
}
fn sha256_cancellable(file: &mut File, cancel: &AtomicBool) -> Result<String> {
    use sha2::{Digest, Sha256};
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        check_job(cancel)?;
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
    let mut original = open_source(input)?;
    // 此流程一律先把來源複製到受控暫存，cjxl／djxl 只拿到副本；
    // 這裡只拒絕無法判定能力的磁碟區。
    filesystem_safety::check(&original).map_err(err)?;
    let destination = Destination::open(dir).map_err(err)?;
    let length = original.metadata().map_err(err)?.len();
    if length > engine_limits::OUTPUT_LIMIT {
        return Err("JPEG ↔ JXL 來源副本超過 16 GiB 上限".into());
    }
    // H1：寫入前先跨執行個體預留。本機暫存同時存放來源副本、本機輸出與（封裝時）還原驗證檔；
    // 目的端只放最終輸出。JPEG 重新封裝通常比原檔小，還原結果與原檔等長，預留取 4 倍與 2 倍。
    let mib64 = 64 * 1024 * 1024;
    let reservations = controlled_temp::preflight_bytes(
        &job_dir(),
        destination.path(),
        length.saturating_mul(4).saturating_add(mib64),
        length.saturating_mul(2).saturating_add(mib64),
        None,
    )
    .map_err(|e| format!("磁碟資源預檢失敗：{e}"))?;
    JOB_RESERVATIONS.with(|r| r.borrow_mut().extend(reservations));
    let mut source = job_file()?;
    copy_bytes(&mut original, &mut source, length, cancel)
        .map_err(|e| copy_failure("來源複製到受控暫存失敗", e))?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    // 輸出與往返驗證都使用本機受控暫存；cjxl／djxl 從不取得目的端控制代碼（H3）。
    let mut output = job_file()?;
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
            &output,
            cancel,
        )?;
        output.seek(SeekFrom::Start(0)).map_err(err)?;
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
            &output,
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
            &output,
            cancel,
        )?;
        hash = sha256(&mut output)?;
        if let Some(path) = reference {
            if sha256(&mut File::open(path).map_err(err)?)? != hash {
                return Err("與參考 JPEG 的 SHA-256 不一致，未保存輸出".into());
            }
            verified = true;
        } else {
            verified = false;
        }
    }
    if output.metadata().map_err(err)?.len() == 0 {
        return Err("輸出為空".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    check_job(cancel)?;
    let result = deliver(
        &mut output,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        if wrap { Format::Jxl } else { Format::Jpeg },
        cancel,
    )?;
    Ok((result, hash, verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn legal_prologue_is_separated_but_diagnostics_are_retained() {
        let start = "IMB-C distribution: linked LGPL libraries (not a Microsoft Runtime license):";
        let end = "Full authors, corresponding source and replacement instructions: ../THIRD-PARTY-RUNTIME.txt and ../LGPL-REPLACEMENT.md.";
        let raw = format!(
            "{start}\r\nGLib license details\r\n{end}\r\nmagick.exe: actual decode error\r\n"
        );
        assert_eq!(diagnostic_text(&raw), "magick.exe: actual decode error");
        assert_eq!(
            diagnostic_text("unknown license warning\nactual error"),
            "unknown license warning\nactual error"
        );
        assert_eq!(
            diagnostic_text(&format!("{start}\ntruncated banner")),
            format!("{start}\ntruncated banner")
        );
    }
    #[test]
    fn verification_line_skips_empty_fields() {
        let base = || Verification {
            lossless_requested: false,
            composite_decoded: false,
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
            "lossy / composite mode; pixels changed; 8→8 bit; ICC preserved; max=0.500000; RMSE=0.250000"
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
            "lossy / composite mode; pixels exact; 8→8 bit; ICC untagged; max=0.500000; RMSE=0.250000; color information not verified"
        );
        let converted = line(Verification {
            color_verified: false,
            transformed: true,
            warnings: vec!["w".into()],
            ..base()
        });
        assert!(converted.starts_with(
            "lossy / composite mode; pixels changed; 8→8 bit; ICC present, not verified; max=0.500000; RMSE=0.250000; compared after requested color/background conversion\n  警告：w"
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
            if !(name.len() + 1).is_multiple_of(2) {
                v.push(0);
            }
            v.extend((data.len() as u32).to_be_bytes());
            v.extend(data);
            if !data.len().is_multiple_of(2) {
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
            include_bytes!("../assets/policy-v14.xml"),
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
        use super::regression_security::{ENGINE_FIXTURE, fixture_exe, fixture_input};
        let exe = fixture_exe();
        let (short, long) = (fixture_input("sleep 500\n"), fixture_input("sleep 5000\n"));
        let output = job_file().unwrap();
        let off = AtomicBool::new(false);
        // Each child has a 10-second allowance, but they share a 3-second job deadline.
        JOB.with(|j| j.borrow_mut().as_mut().unwrap().1 = Instant::now() + Duration::from_secs(3));
        run_with_timeout(
            &exe,
            &ENGINE_FIXTURE,
            &short,
            &output,
            &off,
            Duration::from_secs(10),
        )
        .unwrap();
        let e = run_with_timeout(
            &exe,
            &ENGINE_FIXTURE,
            &long,
            &output,
            &off,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("總期限"), "{e}");
        assert!(
            run(&exe, &ENGINE_FIXTURE, &short, &output, &off)
                .unwrap_err()
                .contains("總期限")
        );
        drop((short, long, output));
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
    #[ignore = "Read-only real PSD sample; set IMBC_TEST_ENGINE and IMBC_TEST_PSD"]
    fn real_csp_psd_conversion_and_source_preservation() {
        let Some(input) = std::env::var_os("IMBC_TEST_PSD")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("IMBC_TEST_ASSETS").map(|p| PathBuf::from(p).join("source.psd"))
            })
        else {
            eprintln!(
                "skipped real_csp_psd_conversion_and_source_preservation: \
                 set IMBC_TEST_PSD to a read-only PSD sample"
            );
            return;
        };
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
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
            let count = fs::read_dir(dir.path()).unwrap().count();
            let result = convert(&engine, &source, dir.path(), format, &off);
            if matches!(format, Format::Psd) {
                assert!(result.unwrap_err().contains("Alpha"));
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
                continue;
            }
            let converted = result.unwrap();
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
        for ext in ["heic", "HEIF"] {
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
        use super::regression_security::{ENGINE_FIXTURE, fixture_exe, fixture_input};
        let parent_temp = std::env::var_os("TEMP");
        for mode in ["success", "failure", "cancel", "timeout"] {
            let action = match mode {
                "success" => "exit 0",
                "failure" => "exit 7",
                _ => "sleep 30000",
            };
            let input = fixture_input(&format!("scratch\n{action}\n"));
            let out_dir = tempfile::tempdir().unwrap();
            let output = tempfile::NamedTempFile::new_in(out_dir.path()).unwrap();
            let cancel = AtomicBool::new(false);
            let result = std::thread::scope(|scope| {
                if mode == "cancel" {
                    let observed = output.path().to_owned();
                    let cancel = &cancel;
                    scope.spawn(move || {
                        let start = Instant::now();
                        while !fs::read_to_string(&observed)
                            .unwrap_or_default()
                            .contains("TEMP_DENIED=")
                            && start.elapsed() < Duration::from_secs(8)
                        {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        cancel.store(true, Ordering::Relaxed);
                    });
                }
                run_with_timeout(
                    &fixture_exe(),
                    &ENGINE_FIXTURE,
                    &input,
                    output.as_file(),
                    &cancel,
                    if mode == "timeout" {
                        Duration::from_secs(3)
                    } else {
                        Duration::from_secs(10)
                    },
                )
            });
            let recorded = fs::read_to_string(output.path()).unwrap();
            let scratch = recorded
                .lines()
                .find_map(|l| l.strip_prefix("SCRATCH="))
                .map(PathBuf::from)
                .unwrap_or_default();
            assert!(
                scratch.is_absolute(),
                "child did not reach scratch fixture: {mode}: {result:?}: {recorded}"
            );
            // AppContainer 的 TEMP 被 Windows 改寫到已刪除的 profile，引擎不能寫入。
            assert!(recorded.contains("TEMP_DENIED=true"), "{mode}: {recorded}");
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
                    let destination = Destination::open(p).unwrap();
                    let mut t = destination.create_temp().unwrap();
                    t.as_file_mut().write_all(b"new").unwrap();
                    publish(
                        &mut t,
                        &destination,
                        std::ffi::OsStr::new("image"),
                        Format::Png,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
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
        let output = tempfile::tempfile().unwrap();
        let input = super::regression_security::fixture_input("sleep 30000\n");
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(500));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(
                run(
                    &super::regression_security::fixture_exe(),
                    &super::regression_security::ENGINE_FIXTURE,
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
            let count = fs::read_dir(dir.path()).unwrap().count();
            let result = convert(&engine, &alpha, dir.path(), format, &off);
            if matches!(format, Format::Psd) {
                assert!(result.unwrap_err().contains("Alpha"));
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
                continue;
            }
            let file = result.unwrap();
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
            let count = fs::read_dir(dir.path()).unwrap().count();
            let result = convert(&engine, &seed, dir.path(), format, &off);
            if matches!(format, Format::Psd) {
                assert!(result.unwrap_err().contains("Alpha"));
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
            } else {
                sources.push(result.unwrap());
            }
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
                let count = fs::read_dir(dir.path()).unwrap().count();
                let result = convert(&engine, &source, dir.path(), format, &off);
                if matches!(format, Format::Psd) && source.extension().unwrap() != "jpg" {
                    assert!(result.unwrap_err().contains("Alpha"));
                    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
                    continue;
                }
                let output = result.unwrap();
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
                            Format::Psb => "PSB",
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

#[cfg(test)]
#[path = "regression_v14.rs"]
mod regression_v14;

#[cfg(test)]
#[path = "regression_rc3.rs"]
mod regression_rc3;
