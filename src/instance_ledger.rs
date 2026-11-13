//! 跨執行個體資源預留（P1-B）與崩潰殘留安全回收（P1-C），2026-11-13。
//!
//! - 每個 IMB-C 程序是一個「執行個體」，在使用者專屬狀態目錄建立 `instances/<id>.journal`，
//!   並以位元組範圍鎖持有到程序結束。鎖由 OS 隨程序終止（含崩潰、被終止）自動釋放，
//!   因此存活判斷不依賴可被重複使用的 PID。
//! - 預留表 `reservations.txt` 只在檔案鎖內讀改寫；已死亡執行個體的預留在下一次存取時移除，
//!   不會永久殘留。預留是協調紀錄（避免多個工作重複計算同一份可用空間／記憶體），
//!   不是檔案系統或記憶體硬配額。
//! - 擁有權日誌記錄本執行個體建立的受控根目錄與目的地暫存檔：磁碟區序號＋128 位元 File ID＋路徑。
//!   回收只處理「已死亡執行個體」日誌中未釋放的項目，且路徑上的物件 File ID 必須與紀錄相同；
//!   不依名稱掃描、不碰共用根目錄的其他項目。
use super::controlled_temp;
use std::{
    collections::HashMap,
    ffi::{OsString, c_void},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

/// 鎖定位置遠超檔尾，不妨礙其他程序讀取日誌內容。
const LIVE_LOCK: u64 = 1 << 62;
const TABLE_LOCK: u64 = 1 << 62;
const LOCKFILE_FAIL_IMMEDIATELY: u32 = 1;
const LOCKFILE_EXCLUSIVE: u32 = 2;
const DELETE: u32 = 0x10000;
const SYNCHRONIZE: u32 = 0x100000;
const READ_ATTRIBUTES: u32 = 0x80;
const LIST_DIRECTORY: u32 = 0x1;
const TRAVERSE: u32 = 0x20;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const BACKUP_SEMANTICS_OPEN_REPARSE: u32 = 0x0220_0000;

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: usize,
}
#[repr(C)]
#[derive(Default)]
struct FileIdInfo {
    volume: u64,
    id: [u8; 16],
}
#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    heap: usize,
    module: u32,
    threads: u32,
    parent: u32,
    priority: i32,
    flags: u32,
    exe: [u16; 260],
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LockFileEx(
        file: *mut c_void,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: *mut c_void,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn GetFileInformationByHandleEx(
        file: *mut c_void,
        class: i32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetProcessTimes(
        process: *mut c_void,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn QueryFullProcessImageNameW(
        process: *mut c_void,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
    fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
}
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *mut c_void, bytes: *mut u8, len: u32, flags: u32) -> i32;
}

fn lock(file: &File, at: u64, flags: u32) -> io::Result<bool> {
    let mut overlapped = Overlapped {
        offset: at as u32,
        offset_high: (at >> 32) as u32,
        ..Default::default()
    };
    if unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &mut overlapped) } != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(33) {
        Ok(false) // ERROR_LOCK_VIOLATION：其他控制代碼持有
    } else {
        Err(error)
    }
}
fn unlock(file: &File, at: u64) {
    let mut overlapped = Overlapped {
        offset: at as u32,
        offset_high: (at >> 32) as u32,
        ..Default::default()
    };
    unsafe { UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped) };
}

/// 磁碟區序號與 128 位元 File ID；任一為 0 視為無法可靠識別。
pub(super) fn file_identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    let mut info = FileIdInfo::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18, // FileIdInfo
            (&mut info as *mut FileIdInfo).cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.volume == 0 || info.id == [0; 16] {
        return Err(io::Error::other("檔案系統未提供可靠的 File ID"));
    }
    Ok((info.volume, info.id))
}

fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, 2) } < 0 {
        return Err(io::Error::other("無法產生執行個體識別碼"));
    }
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}
/// 路徑以 UTF-16 單位十六進位保存，可還原任意 Windows 路徑（含非成對代理）。
fn hex_path(path: &Path) -> String {
    path.as_os_str()
        .encode_wide()
        .map(|u| format!("{u:04x}"))
        .collect()
}
fn unhex_path(text: &str) -> Option<PathBuf> {
    if text.is_empty() || !text.len().is_multiple_of(4) {
        return None;
    }
    let units = (0..text.len())
        .step_by(4)
        .map(|i| u16::from_str_radix(text.get(i..i + 4)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    Some(PathBuf::from(OsString::from_wide(&units)))
}
fn valid_instance_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn process_creation(process: *mut c_void) -> io::Result<u64> {
    let (mut created, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
    if unsafe { GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(created)
}

/// 使用者專屬狀態目錄。正式版以 Known Folder API 取得 LocalAppData，
/// 不採用可被改寫的 LOCALAPPDATA 環境變數；測試使用獨立目錄，不碰使用者實際狀態。
fn state_dir() -> io::Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(std::env::temp_dir().join("imb-c-test-runtime"))
    }
    #[cfg(not(test))]
    {
        #[repr(C)]
        struct Guid {
            data1: u32,
            data2: u16,
            data3: u16,
            data4: [u8; 8],
        }
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHGetKnownFolderPath(
                id: *const Guid,
                flags: u32,
                token: *mut c_void,
                path: *mut *mut u16,
            ) -> i32;
        }
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoTaskMemFree(memory: *mut c_void);
        }
        const LOCAL_APP_DATA: Guid = Guid {
            data1: 0xF1B3_2785,
            data2: 0x6FBA,
            data3: 0x4FCF,
            data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
        };
        let mut raw = std::ptr::null_mut();
        let result =
            unsafe { SHGetKnownFolderPath(&LOCAL_APP_DATA, 0, std::ptr::null_mut(), &mut raw) };
        if result < 0 || raw.is_null() {
            if !raw.is_null() {
                unsafe { CoTaskMemFree(raw.cast()) };
            }
            return Err(io::Error::other(format!(
                "無法定位 LocalAppData（HRESULT 0x{result:08x}）"
            )));
        }
        let mut length = 0;
        while unsafe { *raw.add(length) } != 0 {
            length += 1;
        }
        let path = OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, length) });
        unsafe { CoTaskMemFree(raw.cast()) };
        Ok(PathBuf::from(path).join("IMB-C").join("runtime"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Dir,
    File,
}

pub(super) enum Resource<'a> {
    Memory,
    Disk(&'a str),
}
impl Resource<'_> {
    fn key(&self) -> String {
        match self {
            Resource::Memory => "M".into(),
            Resource::Disk(volume) => format!("D:{}", hex(volume.as_bytes())),
        }
    }
}

struct Line {
    instance: String,
    n: u64,
    key: String,
    bytes: u64,
}

pub(super) struct Ledger {
    dir: PathBuf,
    id: String,
    // None 只出現在測試模擬崩潰（關閉控制代碼＝OS 於程序終止時釋放存活鎖）。
    journal: Mutex<Option<File>>,
    next: AtomicU64,
    outstanding: AtomicI64,
}

pub(super) struct Reservation {
    ledger: Arc<Ledger>,
    n: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let (id, n) = (self.ledger.id.clone(), self.n);
        // 失敗時保留紀錄；執行個體結束後由其他執行個體以存活鎖判定並移除，不會永久殘留。
        let _ = self
            .ledger
            .with_table(|lines| lines.retain(|l| !(l.instance == id && l.n == n)));
    }
}

/// 日誌項目；只在確認已清理或已轉為正式輸出後明確釋放。保留中的項目留給崩潰回收。
pub(super) struct Entry {
    ledger: Arc<Ledger>,
    n: u64,
}
impl Entry {
    pub(super) fn release(self) {
        self.ledger.release(self.n);
    }
}

#[derive(Default, Debug)]
pub(super) struct Recovery {
    pub(super) journals: usize,
    pub(super) removed: usize,
    pub(super) skipped: usize,
    pub(super) orphans: usize,
    pub(super) kept: Vec<String>,
}

impl Ledger {
    pub(super) fn open_in(dir: &Path, engine_dirs: &[PathBuf]) -> io::Result<Self> {
        fs::create_dir_all(dir.join("instances"))?;
        let id = random_id()?;
        let path = dir.join("instances").join(format!("{id}.journal"));
        let mut journal = File::options()
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .create_new(true)
            .share_mode(7)
            .open(&path)?;
        if !lock(
            &journal,
            LIVE_LOCK,
            LOCKFILE_EXCLUSIVE | LOCKFILE_FAIL_IMMEDIATELY,
        )? {
            return Err(io::Error::other("無法取得執行個體存活鎖"));
        }
        let created = process_creation(unsafe { GetCurrentProcess() })?;
        let dirs: Vec<_> = engine_dirs.iter().map(|d| hex_path(d)).collect();
        writeln!(
            journal,
            "IMBC1 {} {created} {}",
            std::process::id(),
            if dirs.is_empty() {
                "-".to_owned()
            } else {
                dirs.join(",")
            }
        )?;
        Ok(Self {
            dir: dir.to_owned(),
            id,
            journal: Mutex::new(Some(journal)),
            next: AtomicU64::new(1),
            outstanding: AtomicI64::new(0),
        })
    }

    fn append(&self, line: &str) -> io::Result<()> {
        let mut journal = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        journal
            .as_mut()
            .ok_or_else(|| io::Error::other("執行個體紀錄已關閉"))?
            .write_all(line.as_bytes())
    }

    #[cfg(test)]
    fn simulate_crash(&self) {
        self.journal.lock().unwrap().take();
    }

    pub(super) fn record(
        self: &Arc<Self>,
        kind: Kind,
        file: &File,
        path: &Path,
    ) -> io::Result<Entry> {
        let (volume, id) = file_identity(file)?;
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        self.append(&format!(
            "+ {n} {} {volume:016x} {} {}\n",
            if kind == Kind::Dir { 'D' } else { 'F' },
            hex(&id),
            hex_path(path)
        ))?;
        self.outstanding.fetch_add(1, Ordering::Relaxed);
        Ok(Entry {
            ledger: self.clone(),
            n,
        })
    }

    fn release(&self, n: u64) {
        if self.append(&format!("- {n}\n")).is_ok() {
            self.outstanding.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn instance_alive(&self, id: &str) -> bool {
        if id == self.id {
            return true;
        }
        if !valid_instance_id(id) {
            return false; // 不合法紀錄不可能對應任何執行個體日誌
        }
        let path = self.dir.join("instances").join(format!("{id}.journal"));
        let file = match File::options().read(true).share_mode(7).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return false,
            Err(_) => return true, // 無法判斷時保守視為存活
        };
        match lock(
            &file,
            LIVE_LOCK,
            LOCKFILE_EXCLUSIVE | LOCKFILE_FAIL_IMMEDIATELY,
        ) {
            Ok(true) => {
                unlock(&file, LIVE_LOCK);
                false
            }
            _ => true,
        }
    }

    fn with_table<T>(&self, f: impl FnOnce(&mut Vec<Line>) -> T) -> io::Result<T> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(7)
            .open(self.dir.join("reservations.txt"))?;
        // 阻塞等待：鎖只在讀改寫期間持有；持有者崩潰時由 OS 釋放，不會死結。
        lock(&file, TABLE_LOCK, LOCKFILE_EXCLUSIVE)?;
        struct Unlock<'a>(&'a File);
        impl Drop for Unlock<'_> {
            fn drop(&mut self) {
                unlock(self.0, TABLE_LOCK);
            }
        }
        let _unlock = Unlock(&file);
        let mut text = String::new();
        (&file).read_to_string(&mut text)?;
        let mut lines: Vec<Line> = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split(' ');
                let line = Line {
                    instance: parts.next()?.to_owned(),
                    n: parts.next()?.parse().ok()?,
                    key: parts.next()?.to_owned(),
                    bytes: parts.next()?.parse().ok()?,
                };
                parts.next().is_none().then_some(line)
            })
            .collect();
        let mut alive = HashMap::new();
        lines.retain(|l| {
            *alive
                .entry(l.instance.clone())
                .or_insert_with(|| self.instance_alive(&l.instance))
        });
        let result = f(&mut lines);
        let mut out = String::new();
        for l in &lines {
            out.push_str(&format!("{} {} {} {}\n", l.instance, l.n, l.key, l.bytes));
        }
        (&file).seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        (&file).write_all(out.as_bytes())?;
        Ok(result)
    }

    /// 在同一把跨程序鎖內：加總所有存活執行個體同類預留，`held + bytes <= capacity` 才登記。
    /// 回傳 `Ok(Err(held))` 表示目前容量不足，其中 `held` 為其他預留總量（0 表示沒有可等待的對象）。
    pub(super) fn try_reserve(
        self: &Arc<Self>,
        resource: Resource,
        bytes: u64,
        capacity: u64,
    ) -> io::Result<Result<Reservation, u64>> {
        let key = resource.key();
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let id = self.id.clone();
        self.with_table(|lines| {
            let held = lines
                .iter()
                .filter(|l| l.key == key)
                .fold(0u64, |sum, l| sum.saturating_add(l.bytes));
            if held
                .checked_add(bytes)
                .is_some_and(|total| total <= capacity)
            {
                lines.push(Line {
                    instance: id,
                    n,
                    key,
                    bytes,
                });
                Ok(())
            } else {
                Err(held)
            }
        })
        .map(|r| {
            r.map(|()| Reservation {
                ledger: self.clone(),
                n,
            })
        })
    }

    /// 回收已死亡執行個體的殘留。存活判斷依日誌存活鎖；回收期間持有該鎖，避免多個執行個體同時處理。
    pub(super) fn recover(&self) -> io::Result<Recovery> {
        let mut report = Recovery::default();
        let instances = self.dir.join("instances");
        for item in fs::read_dir(&instances)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".journal") else {
                continue;
            };
            if id == self.id || !valid_instance_id(id) {
                continue;
            }
            let file = match File::options()
                .access_mode(GENERIC_READ | DELETE)
                .share_mode(7)
                .custom_flags(0x0020_0000) // OPEN_REPARSE_POINT：不跟隨被替換的連結
                .open(item.path())
            {
                Ok(file) => file,
                Err(_) => continue,
            };
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                continue;
            }
            if !lock(
                &file,
                LIVE_LOCK,
                LOCKFILE_EXCLUSIVE | LOCKFILE_FAIL_IMMEDIATELY,
            )? {
                continue; // 仍存活或正由其他執行個體回收
            }
            report.journals += 1;
            let mut text = String::new();
            (&file).take(64 * 1024 * 1024).read_to_string(&mut text)?;
            let mut lines = text.lines();
            let header: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
            if let ["IMBC1", pid, created, dirs] = header[..]
                && let (Ok(pid), Ok(created)) = (pid.parse(), created.parse())
            {
                let dirs: Vec<_> = if dirs == "-" {
                    Vec::new()
                } else {
                    dirs.split(',').filter_map(unhex_path).collect()
                };
                report.orphans += reap_orphans(pid, created, &dirs);
            }
            let mut live: Vec<(u64, Kind, u64, [u8; 16], PathBuf)> = Vec::new();
            for line in lines {
                let parts: Vec<_> = line.split(' ').collect();
                match parts[..] {
                    ["+", n, kind, volume, id, path] => {
                        let parsed = (|| {
                            let id: [u8; 16] = unhex(id)?.try_into().ok()?;
                            Some((
                                n.parse().ok()?,
                                match kind {
                                    "D" => Kind::Dir,
                                    "F" => Kind::File,
                                    _ => return None,
                                },
                                u64::from_str_radix(volume, 16).ok()?,
                                id,
                                unhex_path(path)?,
                            ))
                        })();
                        if let Some(entry) = parsed {
                            live.push(entry);
                        }
                    }
                    ["-", n] => {
                        if let Ok(n) = n.parse::<u64>() {
                            live.retain(|e| e.0 != n);
                        }
                    }
                    _ => (),
                }
            }
            let mut complete = true;
            for (_, kind, volume, id, path) in live {
                match recover_entry(kind, volume, id, &path) {
                    Ok(true) => report.removed += 1,
                    Ok(false) => report.skipped += 1,
                    Err(e) => {
                        complete = false;
                        report.kept.push(format!("{}：{e}", path.display()));
                    }
                }
            }
            if complete {
                // 全部處理完成才刪除日誌；未完成者保留到下次啟動重試。
                controlled_temp::delete_handle(&file)?;
            }
            drop(file); // 關閉即釋放存活鎖
        }
        self.with_table(|_| ())?; // 同時移除已死亡執行個體的預留
        Ok(report)
    }

    /// 正常結束時，若沒有未釋放的日誌項目，移除自身日誌。
    pub(super) fn close_if_clean(&self) {
        if self.outstanding.load(Ordering::Relaxed) == 0 {
            let journal = self.journal.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(journal) = journal.as_ref() {
                let _ = controlled_temp::delete_handle(journal);
            }
        }
    }
}

/// 只在路徑仍是同一物件（File ID 相符）時刪除；任何不符都不刪。
/// 回傳 Ok(true)=已刪除，Ok(false)=不存在或已不是原物件（不動），Err=仍在使用或刪除失敗（保留重試）。
fn recover_entry(kind: Kind, volume: u64, id: [u8; 16], path: &Path) -> io::Result<bool> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Ok(false);
    }
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    // 祖先目錄持有控制代碼且不共享 DELETE：回收期間無法被改名或替換。
    let _ancestors = match controlled_temp::pin_ancestors(parent) {
        Ok(pins) => pins,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let access = DELETE
        | READ_ATTRIBUTES
        | SYNCHRONIZE
        | if kind == Kind::Dir {
            LIST_DIRECTORY | TRAVERSE
        } else {
            0
        };
    let opened = File::options()
        .access_mode(access)
        // 目錄只共享 READ（同 ControlledDir）；檔案完全不共享：仍被其他程序開啟就失敗並保留。
        .share_mode(if kind == Kind::Dir { 1 } else { 0 })
        .custom_flags(BACKUP_SEMANTICS_OPEN_REPARSE)
        .open(path);
    let handle = match opened {
        Ok(handle) => handle,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io::Error::new(e.kind(), format!("仍在使用或無法開啟：{e}"))),
    };
    let meta = handle.metadata()?;
    if meta.file_attributes() & 0x400 != 0 || meta.is_dir() != (kind == Kind::Dir) {
        return Ok(false);
    }
    match file_identity(&handle) {
        Ok(actual) if actual == (volume, id) => (),
        _ => return Ok(false), // 已不是 IMB-C 建立的那個物件：絕不刪除
    }
    if kind == Kind::Dir {
        controlled_temp::remove_children(path, &handle)?;
    }
    controlled_temp::delete_handle(&handle)?;
    Ok(true)
}

/// 建立→納管空窗的暫停孤兒：父程序是已死亡執行個體（PID 與建立時間吻合），
/// 映像位於該執行個體紀錄的私有 runtime 目錄。只終止符合全部條件的程序。
fn reap_orphans(pid: u32, created: u64, dirs: &[PathBuf]) -> usize {
    if dirs.is_empty() {
        return 0;
    }
    let dirs: Vec<String> = dirs
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .map(|d| d.to_string_lossy().to_lowercase())
        .collect();
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) }; // TH32CS_SNAPPROCESS
    if snapshot as isize == -1 {
        return 0;
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = ProcessEntry {
        size: std::mem::size_of::<ProcessEntry>() as u32,
        usage: 0,
        pid: 0,
        heap: 0,
        module: 0,
        threads: 0,
        parent: 0,
        priority: 0,
        flags: 0,
        exe: [0; 260],
    };
    let mut children = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while more != 0 {
        if entry.parent == pid && entry.pid != 0 {
            children.push(entry.pid);
        }
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    // PID 已被新程序重用時，新程序建立後才出現的子程序屬於新程序，不得處理。
    let reused_after =
        open_process(pid, 0x1000).and_then(|h| process_creation(h.as_raw_handle()).ok());
    let mut killed = 0;
    for child in children {
        let Some(handle) = open_process(child, 0x1000 | 0x1 | 0x100000) else {
            continue;
        };
        let Ok(child_created) = process_creation(handle.as_raw_handle()) else {
            continue;
        };
        if child_created < created
            || reused_after.is_some_and(|t| t > created && child_created >= t)
        {
            continue;
        }
        let mut name = [0u16; 32768];
        let mut size = name.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(handle.as_raw_handle(), 0, name.as_mut_ptr(), &mut size)
        } == 0
        {
            continue;
        }
        let image = PathBuf::from(OsString::from_wide(&name[..size as usize]));
        let Some(home) = image
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        else {
            continue;
        };
        if !dirs.contains(&home.to_string_lossy().to_lowercase()) {
            continue;
        }
        if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } != 0 {
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) };
            killed += 1;
        }
    }
    killed
}
fn open_process(pid: u32, access: u32) -> Option<OwnedHandle> {
    let handle = unsafe { OpenProcess(access, 0, pid) };
    (!handle.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(handle) })
}

static GLOBAL: OnceLock<Result<Arc<Ledger>, String>> = OnceLock::new();

/// 本程序的執行個體紀錄；第一次使用時建立。紀錄無法建立時拒絕工作（不默默失去協調與回收）。
pub(super) fn global() -> io::Result<Arc<Ledger>> {
    GLOBAL
        .get_or_init(|| {
            let mut dirs = Vec::new();
            if let Some(home) = super::locate_engine().parent() {
                dirs.push(home.to_path_buf());
            }
            if let Ok(tools) = super::locate_jxl_tools(std::env::current_exe()) {
                dirs.push(tools);
            }
            dirs.retain(|d| d.is_dir());
            state_dir()
                .and_then(|dir| Ledger::open_in(&dir, &dirs))
                .map(Arc::new)
                .map_err(|e| {
                    format!("無法建立 IMB-C 執行個體紀錄（跨執行個體預留與崩潰回收需要）：{e}")
                })
        })
        .clone()
        .map_err(io::Error::other)
}

pub(super) fn shutdown() {
    if let Some(Ok(ledger)) = GLOBAL.get() {
        ledger.close_if_clean();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated() -> (tempfile::TempDir, Arc<Ledger>) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Arc::new(Ledger::open_in(dir.path(), &[]).unwrap());
        (dir, ledger)
    }

    #[test]
    fn sdk_layouts() {
        assert_eq!(std::mem::size_of::<Overlapped>(), 32);
        assert_eq!(std::mem::size_of::<FileIdInfo>(), 24);
        assert_eq!(std::mem::size_of::<ProcessEntry>(), 568);
    }

    #[test]
    fn path_and_identity_encoding_round_trip() {
        let path = PathBuf::from(OsString::from_wide(&[
            0x44, 0x3a, 0x5c, 0x4e2d, 0xd800, 0x41,
        ]));
        assert_eq!(unhex_path(&hex_path(&path)).unwrap(), path);
        assert!(valid_instance_id(&random_id().unwrap()));
        assert!(!valid_instance_id("..\\..\\x"));
    }

    // P1-B：兩個執行個體看同一份容量；第二個不能重複使用第一個已預留的部分。
    #[test]
    fn two_instances_cannot_double_book_the_same_capacity() {
        let (dir, a) = isolated();
        let b = Arc::new(Ledger::open_in(dir.path(), &[]).unwrap());
        let first = a
            .try_reserve(Resource::Disk("vol-1"), 60, 100)
            .unwrap()
            .unwrap();
        assert_eq!(
            b.try_reserve(Resource::Disk("vol-1"), 60, 100)
                .unwrap()
                .err(),
            Some(60)
        );
        // 不同磁碟區分開計算。
        let other = b
            .try_reserve(Resource::Disk("vol-2"), 60, 100)
            .unwrap()
            .unwrap();
        drop(first);
        let second = b
            .try_reserve(Resource::Disk("vol-1"), 60, 100)
            .unwrap()
            .unwrap();
        drop((other, second));
        assert_eq!(
            a.try_reserve(Resource::Memory, 10, 9).unwrap().err(),
            Some(0)
        );
    }

    // 同時競爭：多執行緒、多個執行個體交錯預留，總量永不超過容量。
    #[test]
    fn concurrent_instances_never_exceed_capacity() {
        let (dir, _) = isolated();
        let ledgers: Vec<_> = (0..4)
            .map(|_| Arc::new(Ledger::open_in(dir.path(), &[]).unwrap()))
            .collect();
        // 所有核准的預留都持有到全部執行緒結束，所以總和必須 <= 50（最多 7 筆 × 7）。
        let held: Vec<Reservation> = std::thread::scope(|scope| {
            let handles: Vec<_> = ledgers
                .iter()
                .map(|ledger| {
                    scope.spawn(move || {
                        (0..20)
                            .filter_map(|_| {
                                ledger.try_reserve(Resource::Memory, 7, 50).unwrap().ok()
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        assert_eq!(held.len(), 7, "granted {} × 7 bytes", held.len());
        drop(held);
        assert!(
            ledgers[0]
                .try_reserve(Resource::Memory, 50, 50)
                .unwrap()
                .is_ok()
        );
    }

    // P1-B／P1-C：模擬崩潰（不執行 Drop，直接關閉日誌控制代碼釋放存活鎖），
    // 其預留在下一次存取時被移除，不會永久佔用。
    #[test]
    fn crashed_instance_reservations_are_reclaimed() {
        let (dir, a) = isolated();
        let b = Ledger::open_in(dir.path(), &[]).unwrap();
        let b = Arc::new(b);
        let held = b.try_reserve(Resource::Memory, 80, 100).unwrap().unwrap();
        std::mem::forget(held); // 崩潰：沒有釋放
        assert_eq!(
            a.try_reserve(Resource::Memory, 80, 100).unwrap().err(),
            Some(80)
        );
        // 關閉 b 的日誌控制代碼 == 程序終止時 OS 釋放存活鎖。
        b.simulate_crash();
        assert!(a.try_reserve(Resource::Memory, 80, 100).unwrap().is_ok());
    }

    // P1-C：只回收死亡執行個體日誌中、File ID 相符的項目；同名但被替換的物件與使用者檔案不動。
    #[test]
    fn recovery_removes_only_recorded_objects_of_dead_instances() {
        let (dir, survivor) = isolated();
        let root = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
        let crashed = Arc::new(Ledger::open_in(dir.path(), &[]).unwrap());
        // 1) 已記錄的工作目錄（內含子目錄與檔案）
        let work = root.path().join("imb-c-work-recorded");
        fs::create_dir(&work).unwrap();
        fs::create_dir(work.join("nested")).unwrap();
        fs::write(work.join("nested").join("cache"), b"x").unwrap();
        let handle = File::options()
            .access_mode(READ_ATTRIBUTES)
            .custom_flags(BACKUP_SEMANTICS_OPEN_REPARSE)
            .open(&work)
            .unwrap();
        crashed.record(Kind::Dir, &handle, &work).unwrap();
        drop(handle);
        // 2) 已記錄、但之後被替換成同名使用者檔案（File ID 不同）
        let replaced = root.path().join(".tmpREPLACED");
        fs::write(&replaced, b"temp").unwrap();
        let handle = File::open(&replaced).unwrap();
        crashed.record(Kind::File, &handle, &replaced).unwrap();
        drop(handle);
        fs::remove_file(&replaced).unwrap();
        fs::write(&replaced, b"user data").unwrap();
        // 3) 已記錄且已釋放（正常完成）的檔案：不得刪除
        let released = root.path().join("published.png");
        fs::write(&released, b"output").unwrap();
        let handle = File::open(&released).unwrap();
        crashed
            .record(Kind::File, &handle, &released)
            .unwrap()
            .release();
        drop(handle);
        // 4) 未記錄、只是名稱相似的目錄：不得刪除
        let lookalike = root.path().join("imb-c-work-unrecorded");
        fs::create_dir(&lookalike).unwrap();
        // 存活中的執行個體不得被回收
        assert_eq!(survivor.recover().unwrap().journals, 0);
        // 模擬崩潰
        crashed.simulate_crash();
        let report = survivor.recover().unwrap();
        assert_eq!(report.journals, 1, "{report:?}");
        assert_eq!(report.removed, 1, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        assert!(report.kept.is_empty(), "{report:?}");
        assert!(!work.exists());
        assert_eq!(fs::read(&replaced).unwrap(), b"user data");
        assert_eq!(fs::read(&released).unwrap(), b"output");
        assert!(lookalike.is_dir());
        // 日誌完成後已刪除，再次回收無事可做。
        assert_eq!(survivor.recover().unwrap().journals, 0);
    }

    // 回收時仍被其他程序持有的項目：保留並保留日誌，下一次再試。
    #[test]
    fn recovery_keeps_entries_still_in_use() {
        let (dir, survivor) = isolated();
        let root = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
        let crashed = Arc::new(Ledger::open_in(dir.path(), &[]).unwrap());
        let file = root.path().join(".tmpINUSE");
        fs::write(&file, b"partial").unwrap();
        let holder = File::open(&file).unwrap();
        crashed.record(Kind::File, &holder, &file).unwrap();
        crashed.simulate_crash();
        let report = survivor.recover().unwrap();
        assert_eq!(report.kept.len(), 1, "{report:?}");
        assert!(file.exists());
        drop(holder);
        let report = survivor.recover().unwrap();
        assert_eq!((report.journals, report.removed), (1, 1), "{report:?}");
        assert!(!file.exists());
    }
}
