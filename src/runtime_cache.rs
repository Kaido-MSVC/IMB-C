//! 每使用者、依版本與內嵌清單雜湊隔離的本機 NTFS runtime 快取（RC2 H2）。
//!
//! 引擎在 AppContainer 內只能讀取明確授權的位置。舊做法直接替 Portable 解壓目錄遞迴加入
//! capability ACE（永久修改使用者資料夾、唯讀目錄無法使用、樹內其他檔案也一併開放）。
//! 現在改為：
//! - 只複製內嵌清單（`RUNTIME-MANIFEST-v1.4.json`）中 `ImageMagick/`、`JXL/` 的檔案，
//!   邊複製邊計算 SHA-256，與內嵌雜湊不符即拒絕；Portable 目錄只讀不寫。
//! - 快取位於 `%LOCALAPPDATA%\IMB-C\engine-runtime\<版本>-<清單雜湊>`；版本或內容改變即使用新目錄，
//!   不沿用舊檔案或舊 ACL。
//! - 每個檔案與目錄各自設定受保護（不繼承）的 DACL：使用者只有讀取／執行／刪除、SYSTEM 完全控制、
//!   自訂 capability 只有讀取／執行；沒有任何可繼承 ACE，也沒有任何主體取得寫入權限（SYSTEM 除外）。
//! - 使用前以控制代碼逐一開啟並鎖定（檔案不共享寫入與刪除、目錄不共享刪除），再核對內容雜湊、
//!   DACL、硬連結數、重新解析點、實際路徑與目錄內容；鎖定持續到程序結束，其他工作或執行個體
//!   無法在使用期間改寫、改名或替換。
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Handle = *mut c_void;

/// 建置時內嵌的 runtime 清單；與 Portable 目錄內的副本無關，避免以磁碟上的清單自我背書。
const MANIFEST: &str = include_str!("../RUNTIME-MANIFEST-v1.4.json");
/// 只有這兩個子目錄會被引擎載入；授權文件、來源封存等不進入快取。
const COMPONENTS: [&str; 2] = ["ImageMagick/", "JXL/"];

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const LIST: u32 = 0x1;
const ADD_SUBDIRECTORY: u32 = 0x4;
const TRAVERSE: u32 = 0x20;
const READ_ATTRIBUTES: u32 = 0x80;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_ATTRIBUTE: u32 = 0x400;
/// 使用者：讀取、執行、刪除（供快取清理）；不含任何寫入。
const FILE_USER_MASK: u32 = 0x0013_00a9;
/// 使用者：列舉、周遊、刪除、刪除子項；不含新增檔案或子目錄。
const DIR_USER_MASK: u32 = 0x0013_00e9;
/// capability：FILE_GENERIC_READ | FILE_EXECUTE（目錄為列舉與周遊）。
const CAPABILITY_MASK: u32 = 0x0012_00a9;

#[repr(C)]
struct FileInfo {
    attributes: u32,
    times: [u32; 6],
    serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}
#[repr(C)]
struct RenameInfo {
    replace: u32,
    root: Handle,
    length: u32,
    name: [u16; 260],
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetSecurityInfo(
        handle: Handle,
        kind: u32,
        information: u32,
        owner: *mut Handle,
        group: *mut Handle,
        dacl: *mut Handle,
        sacl: *mut Handle,
        descriptor: *mut Handle,
    ) -> u32;
    fn SetSecurityInfo(
        handle: Handle,
        kind: u32,
        information: u32,
        owner: Handle,
        group: Handle,
        dacl: Handle,
        sacl: Handle,
    ) -> u32;
    fn GetSecurityDescriptorControl(
        descriptor: Handle,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: Handle,
        present: *mut i32,
        acl: *mut Handle,
        defaulted: *mut i32,
    ) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        data: Handle,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetInformationFile(
        file: Handle,
        status: *mut c_void,
        information: *const c_void,
        length: u32,
        class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(file: Handle, info: *mut FileInfo) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn LocalFree(pointer: Handle) -> Handle;
}

struct Local(Handle);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[derive(Clone)]
struct Entry {
    rel: String,
    hash: [u8; 32],
    source: PathBuf,
}
struct Spec {
    key: String,
    entries: Vec<Entry>,
}

/// 已驗證並鎖定的快取；鎖定控制代碼存活到程序結束。
struct Prepared {
    key: String,
    root: PathBuf,
    files: BTreeMap<String, PathBuf>,
    _pins: Vec<File>,
}
static PREPARED: Mutex<Vec<Arc<Prepared>>> = Mutex::new(Vec::new());
/// 已清理過的快取根目錄（每個根目錄每個程序只清理一次）。
static CLEANED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn norm(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
fn valid_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn production_entries(base: &Path) -> io::Result<Vec<Entry>> {
    let map: BTreeMap<String, String> = serde_json::from_str(MANIFEST)
        .map_err(|e| io::Error::other(format!("內嵌 runtime 清單無效：{e}")))?;
    let mut entries = Vec::new();
    for (rel, hash) in map {
        if !COMPONENTS.iter().any(|c| rel.starts_with(c)) {
            continue;
        }
        let digest = unhex32(&hash.to_ascii_lowercase())
            .ok_or_else(|| io::Error::other(format!("內嵌 runtime 清單雜湊無效：{rel}")))?;
        if !valid_rel(&rel) {
            return Err(io::Error::other(format!(
                "內嵌 runtime 清單路徑無效：{rel}"
            )));
        }
        entries.push(Entry {
            source: base.join(&rel),
            rel,
            hash: digest,
        });
    }
    Ok(entries)
}
fn production_key() -> String {
    format!(
        "{}-{}",
        env!("CARGO_PKG_VERSION"),
        &hex(&Sha256::digest(MANIFEST.as_bytes()))[..16]
    )
}

/// 快取根目錄：`%LOCALAPPDATA%\IMB-C\engine-runtime`（測試改用系統暫存下的獨立目錄）。
pub(super) fn cache_root() -> io::Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(std::env::temp_dir().join("imb-c-test-engine-runtime"))
    }
    #[cfg(not(test))]
    {
        Ok(super::instance_ledger::app_data_dir()?.join("engine-runtime"))
    }
}

/// 正式 runtime 快取中的 ImageMagick／JXL 目錄（供崩潰回收辨識暫停的引擎程序）。
pub(super) fn production_component_dirs() -> Vec<PathBuf> {
    let Ok(root) = cache_root() else {
        return Vec::new();
    };
    let key = production_key();
    COMPONENTS
        .iter()
        .map(|c| root.join(&key).join(c.trim_end_matches('/')))
        .collect()
}

/// 將引擎路徑對應到已驗證、已鎖定的快取副本；必要時建立快取。
/// 已是快取內檔案時原樣回傳。不在內嵌清單內的執行檔一律拒絕（測試建置另以測試程式本身建立快取）。
pub(super) fn resolve(engine: &Path) -> io::Result<PathBuf> {
    let canonical = engine
        .canonicalize()
        .map_err(|e| io::Error::new(e.kind(), format!("引擎路徑解析失敗：{e}")))?;
    let mut prepared = PREPARED
        .lock()
        .map_err(|_| io::Error::other("runtime 快取狀態不可用"))?;
    let wanted = norm(&canonical);
    if let Some(path) = prepared.iter().find_map(|p| p.files.get(&wanted)) {
        return Ok(path.clone());
    }
    let (spec, rel) = spec_for(&canonical)?;
    let entry = match prepared.iter().find(|p| p.key == spec.key) {
        Some(existing) => existing.clone(),
        None => {
            let root = cache_root()?;
            let created = Arc::new(prepare(&root, &spec)?);
            prepared.push(created.clone());
            created
        }
    };
    Ok(entry.root.join(rel.replace('/', "\\")))
}

fn spec_for(engine: &Path) -> io::Result<(Spec, String)> {
    let parent = engine.parent();
    let base = parent.and_then(Path::parent);
    if let (Some(parent), Some(base), Some(name)) = (parent, base, engine.file_name()) {
        let rel = format!(
            "{}/{}",
            parent.file_name().unwrap_or_default().to_string_lossy(),
            name.to_string_lossy()
        );
        let entries = production_entries(base)?;
        if let Some(entry) = entries.iter().find(|e| e.rel.eq_ignore_ascii_case(&rel)) {
            let rel = entry.rel.clone();
            return Ok((
                Spec {
                    key: production_key(),
                    entries,
                },
                rel,
            ));
        }
    }
    #[cfg(test)]
    {
        // 測試建置：以測試程式本身作為 fixture 引擎，同樣走「複製→雜湊→鎖定→驗證」流程。
        let mut file = File::open(engine)?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        let hash: [u8; 32] = hasher.finalize().into();
        let name = engine
            .file_name()
            .ok_or_else(|| io::Error::other("fixture 名稱無效"))?
            .to_string_lossy()
            .into_owned();
        let rel = format!("fixture/{name}");
        Ok((
            Spec {
                key: format!("fixture-{}", &hex(&hash)[..16]),
                entries: vec![Entry {
                    rel: rel.clone(),
                    hash,
                    source: engine.to_owned(),
                }],
            },
            rel,
        ))
    }
    #[cfg(not(test))]
    Err(io::Error::other(format!(
        "{} 不在 IMB-C runtime 清單內，已拒絕啟動",
        engine.display()
    )))
}

enum Problem {
    Missing,
    Invalid(String),
    Io(io::Error),
}
impl From<io::Error> for Problem {
    fn from(e: io::Error) -> Self {
        Problem::Io(e)
    }
}

fn open_dir(path: &Path, access: u32, share: u32) -> io::Result<File> {
    let dir = OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = dir.metadata()?;
    if !meta.is_dir() || meta.file_attributes() & REPARSE_ATTRIBUTE != 0 {
        return Err(io::Error::other(format!(
            "{} 不是一般目錄（重新解析點或檔案），已拒絕",
            path.display()
        )));
    }
    Ok(dir)
}
fn random_name(prefix: &str) -> io::Result<String> {
    Ok(format!("{prefix}{}", super::instance_ledger::random_id()?))
}
fn rename_relative(file: &File, root: &File, name: &str) -> io::Result<()> {
    let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().collect();
    if wide.len() >= 260 || wide.contains(&92) || wide.contains(&47) {
        return Err(io::Error::other("快取名稱無效"));
    }
    let mut info = RenameInfo {
        replace: 0,
        root: root.as_raw_handle(),
        length: (wide.len() * 2) as u32,
        name: [0; 260],
    };
    info.name[..wide.len()].copy_from_slice(&wide);
    // FILE_RENAME_INFORMATION（RootDirectory 相對）。SetFileInformationByHandle 對相對
    // RootDirectory 回傳 ERROR_INVALID_PARAMETER（實測），改直接呼叫 NtSetInformationFile。
    let mut status = [0usize; 2];
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            status.as_mut_ptr().cast(),
            (&info as *const RenameInfo).cast(),
            std::mem::size_of::<RenameInfo>() as u32,
            10, // FileRenameInformation
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    Ok(())
}

fn user_sid() -> io::Result<String> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    struct Token(Handle);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let token = Token(token);
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, 1, std::ptr::null_mut(), 0, &mut size);
    }
    if (size as usize) < std::mem::size_of::<usize>() * 2 {
        return Err(io::Error::other("Token 使用者大小錯誤"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, 1, data.as_mut_ptr().cast(), size, &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(data[0] as Handle, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let owner = Local(text.cast());
    let value = unsafe {
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    drop(owner);
    Ok(value)
}
fn sddl(user: &str, directory: bool) -> String {
    format!(
        "D:P(A;;{:#x};;;{user})(A;;FA;;;SY)(A;;{CAPABILITY_MASK:#x};;;{})",
        if directory {
            DIR_USER_MASK
        } else {
            FILE_USER_MASK
        },
        super::low_integrity::RUNTIME_CAPABILITY
    )
}
struct Descriptor(Local);
impl Descriptor {
    fn parse(text: &str) -> io::Result<Self> {
        let wide: Vec<u16> = std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut sd = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(Local(sd)))
    }
    fn dacl(&self) -> io::Result<Handle> {
        let (mut present, mut defaulted, mut acl) = (0, 0, std::ptr::null_mut());
        if unsafe { GetSecurityDescriptorDacl(self.0.0, &mut present, &mut acl, &mut defaulted) }
            == 0
            || present == 0
            || acl.is_null()
        {
            return Err(io::Error::other("DACL 建立失敗"));
        }
        Ok(acl)
    }
}
/// ACL 內各 ACE 的位元組（不含 ACL 標頭中可能較大的配置長度）。
fn ace_bytes(acl: Handle) -> Vec<Vec<u8>> {
    let count = unsafe { *(acl.cast::<u8>().add(4).cast::<u16>()) } as usize;
    let mut aces = Vec::with_capacity(count);
    let mut at = unsafe { acl.cast::<u8>().add(8) };
    for _ in 0..count {
        let size = unsafe { *(at.add(2).cast::<u16>()) } as usize;
        aces.push(unsafe { std::slice::from_raw_parts(at, size) }.to_vec());
        at = unsafe { at.add(size) };
    }
    aces
}
fn set_dacl(file: &File, expected: &Descriptor) -> io::Result<()> {
    let error = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            1,
            0x8000_0004, // PROTECTED_DACL | DACL
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            expected.dacl()?,
            std::ptr::null_mut(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
}
fn dacl_matches(file: &File, expected: &Descriptor) -> io::Result<bool> {
    let (mut dacl, mut descriptor) = (std::ptr::null_mut(), std::ptr::null_mut());
    let error = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            4,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let descriptor = Local(descriptor);
    let (mut control, mut revision) = (0u16, 0u32);
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(control & 0x1000 != 0 // SE_DACL_PROTECTED
        && !dacl.is_null()
        && ace_bytes(dacl) == ace_bytes(expected.dacl()?))
}
fn link_count(file: &File) -> io::Result<u32> {
    let mut info = std::mem::MaybeUninit::<FileInfo>::zeroed();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init() }.links)
}

/// 快取內的目錄（相對路徑，"" 為版本目錄本身），父目錄在前。
fn directories(entries: &[Entry]) -> Vec<String> {
    let mut dirs = BTreeSet::new();
    dirs.insert(String::new());
    for entry in entries {
        let mut current = String::new();
        let parts: Vec<_> = entry.rel.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            dirs.insert(current.clone());
        }
    }
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.sort_by_key(|d| {
        (
            d.matches('/').count() + usize::from(!d.is_empty()),
            d.clone(),
        )
    });
    dirs
}
fn children(dir: &str, entries: &[Entry], dirs: &[String]) -> BTreeSet<String> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let direct = |rel: &str| {
        rel.strip_prefix(&prefix)
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .map(str::to_lowercase)
    };
    entries
        .iter()
        .filter_map(|e| direct(&e.rel))
        .chain(dirs.iter().filter_map(|d| direct(d)))
        .collect()
}
fn rel_path(root: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        root.to_owned()
    } else {
        root.join(rel.replace('/', "\\"))
    }
}

fn prepare(root: &Path, spec: &Spec) -> io::Result<Prepared> {
    fs::create_dir_all(root)?;
    // 根目錄與所有祖先不共享刪除：使用期間無法被改名或替換成連結。
    let mut pins = super::controlled_temp::pin_ancestors(root)?;
    let root_handle = open_dir(root, LIST | ADD_SUBDIRECTORY | TRAVERSE | SYNCHRONIZE, 3)?;
    match super::filesystem_safety::check_actual(&root_handle)? {
        super::filesystem_safety::Protection::LocalNtfs => {}
        super::filesystem_safety::Protection::CopyFirst(fs) => {
            return Err(io::Error::other(format!(
                "runtime 快取位置 {} 不是本機 NTFS（{fs}），無法以 ACL 隔離引擎，已拒絕",
                root.display()
            )));
        }
    }
    let root = super::filesystem_safety::final_path(&root_handle)?;
    let user = user_sid()?;
    let file_sd = Descriptor::parse(&sddl(&user, false))?;
    let dir_sd = Descriptor::parse(&sddl(&user, true))?;
    if CLEANED
        .lock()
        .map_err(|_| io::Error::other("runtime 快取狀態不可用"))?
        .insert(norm(&root))
    {
        cleanup(&root, &spec.key, &file_sd);
    }
    let mut last = String::new();
    for _ in 0..3 {
        match open_existing(&root, spec, &file_sd, &dir_sd) {
            Ok((files, handles)) => {
                pins.push(root_handle);
                pins.extend(handles);
                return Ok(Prepared {
                    key: spec.key.clone(),
                    root: root.join(&spec.key),
                    files,
                    _pins: pins,
                });
            }
            Err(Problem::Missing) => build(&root, &root_handle, spec, &file_sd, &dir_sd)?,
            Err(Problem::Invalid(reason)) => {
                last = reason.clone();
                discard(&root, &root_handle, &spec.key, &file_sd).map_err(|e| {
                    io::Error::other(format!(
                        "runtime 快取 {} 驗證失敗（{reason}），且無法安全移除：{e}",
                        root.join(&spec.key).display()
                    ))
                })?;
            }
            Err(Problem::Io(e)) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "runtime 快取重建後仍無法驗證：{last}"
    )))
}

fn open_existing(
    root: &Path,
    spec: &Spec,
    file_sd: &Descriptor,
    dir_sd: &Descriptor,
) -> Result<(BTreeMap<String, PathBuf>, Vec<File>), Problem> {
    let key_dir = root.join(&spec.key);
    let dirs = directories(&spec.entries);
    let mut handles = Vec::new();
    let mut files = BTreeMap::new();
    for dir in &dirs {
        let path = rel_path(&key_dir, dir);
        // 不共享刪除：持有期間無法被改名或刪除。
        let handle = match open_dir(&path, READ_CONTROL | LIST | TRAVERSE | READ_ATTRIBUTES, 3) {
            Ok(handle) => handle,
            Err(e) if dir.is_empty() && e.kind() == io::ErrorKind::NotFound => {
                return Err(Problem::Missing);
            }
            Err(e) => return Err(Problem::Invalid(format!("{}：{e}", path.display()))),
        };
        if norm(&super::filesystem_safety::final_path(&handle)?) != norm(&path) {
            return Err(Problem::Invalid(format!("{} 實際位置不符", path.display())));
        }
        if !dacl_matches(&handle, dir_sd)? {
            return Err(Problem::Invalid(format!("{} 權限不符", path.display())));
        }
        let mut actual = BTreeSet::new();
        for item in fs::read_dir(&path)? {
            actual.insert(item?.file_name().to_string_lossy().to_lowercase());
        }
        if actual != children(dir, &spec.entries, &dirs) {
            return Err(Problem::Invalid(format!(
                "{} 內容與清單不符",
                path.display()
            )));
        }
        handles.push(handle);
    }
    for entry in &spec.entries {
        let path = rel_path(&key_dir, &entry.rel);
        // 只共享讀取：持有期間任何程序都無法以寫入或刪除權限開啟、改名或替換。
        let mut file = OpenOptions::new()
            .access_mode(GENERIC_READ | READ_CONTROL)
            .share_mode(1)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|e| Problem::Invalid(format!("{}：{e}", path.display())))?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.file_attributes() & REPARSE_ATTRIBUTE != 0 {
            return Err(Problem::Invalid(format!("{} 不是一般檔案", path.display())));
        }
        if link_count(&file)? != 1 {
            return Err(Problem::Invalid(format!("{} 有多個硬連結", path.display())));
        }
        if norm(&super::filesystem_safety::final_path(&file)?) != norm(&path) {
            return Err(Problem::Invalid(format!("{} 實際位置不符", path.display())));
        }
        if !dacl_matches(&file, file_sd)? {
            return Err(Problem::Invalid(format!("{} 權限不符", path.display())));
        }
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        if <[u8; 32]>::from(hasher.finalize()) != entry.hash {
            return Err(Problem::Invalid(format!("{} 內容雜湊不符", path.display())));
        }
        files.insert(norm(&path), path);
        handles.push(file);
    }
    Ok((files, handles))
}

fn build(
    root: &Path,
    root_handle: &File,
    spec: &Spec,
    file_sd: &Descriptor,
    dir_sd: &Descriptor,
) -> io::Result<()> {
    let (staging_name, staging_handle) =
        super::controlled_temp::create_directory(root_handle, "staging-", 3)?;
    let staging = root.join(&staging_name);
    let mut registered = false;
    let mut created = Vec::new();
    let result = (|| {
        // 建立紀錄：只有登錄過、身分相符的目錄日後才可能被回收（第三輪）。
        {
            let mut registry = Registry::lock(root)?;
            let (volume, id) = super::instance_ledger::file_identity(&staging_handle)?;
            registry.set(Owned {
                name: staging_name.clone(),
                kind: "staging".into(),
                volume,
                id,
                children: Vec::new(),
            });
            registry.save()?;
            registered = true;
        }
        let dirs = directories(&spec.entries);
        for dir in dirs.iter().filter(|d| !d.is_empty()) {
            let path = rel_path(&staging, dir);
            fs::create_dir(&path)?;
            let handle = open_dir(&path, LIST | READ_ATTRIBUTES, 3)?;
            created.push((
                format!("{dir}/"),
                super::instance_ledger::file_identity(&handle)?.1,
            ));
        }
        for entry in &spec.entries {
            // Portable 目錄只讀：拒絕同時有寫入者的來源，避免複製到一半被改。
            let mut source = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&entry.source)
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!(
                            "無法讀取 runtime 檔案 {}：{e}；請確認 IMB-C 已完整解壓",
                            entry.source.display()
                        ),
                    )
                })?;
            if !source.metadata()?.is_file() {
                return Err(io::Error::other(format!(
                    "runtime 項目 {} 不是一般檔案",
                    entry.source.display()
                )));
            }
            let target = rel_path(&staging, &entry.rel);
            let mut copy = OpenOptions::new()
                .access_mode(GENERIC_READ | GENERIC_WRITE | READ_CONTROL | WRITE_DAC)
                .share_mode(0)
                .create_new(true)
                .open(&target)?;
            created.push((
                entry.rel.clone(),
                super::instance_ledger::file_identity(&copy)?.1,
            ));
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 1 << 16];
            loop {
                let n = source.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                copy.write_all(&buffer[..n])?;
            }
            if <[u8; 32]>::from(hasher.finalize()) != entry.hash {
                return Err(io::Error::other(format!(
                    "runtime 檔案 {} 與此版本內嵌清單的 SHA-256 不符；請重新解壓完整的 IMB-C，勿混用不同版本的 runtime",
                    entry.source.display()
                )));
            }
            copy.sync_all()?;
            set_dacl(&copy, file_sd)?;
        }
        // 目錄由深到淺收緊；版本目錄本身最後處理。
        for dir in dirs.iter().rev().filter(|d| !d.is_empty()) {
            let handle = open_dir(&rel_path(&staging, dir), READ_CONTROL | WRITE_DAC, 7)?;
            set_dacl(&handle, dir_sd)?;
        }
        set_dacl(&staging_handle, dir_sd)?;
        // 改名與登錄更新在同一把登錄鎖內完成，回收者不會看到不一致的狀態。
        let mut registry = Registry::lock(root)?;
        if let Some(mut record) = registry.take(&staging_name) {
            record.children.clone_from(&created);
            registry.set(record);
            registry.save()?;
        }
        match rename_relative(&staging_handle, root_handle, &spec.key) {
            Ok(()) => {
                if let Some(record) = registry.take(&staging_name) {
                    registry.set(Owned {
                        name: spec.key.clone(),
                        kind: "cache".into(),
                        ..record
                    });
                }
                registry.save()?;
                Ok(true)
            }
            // 其他執行個體已先完成同一版本：改用它的（之後同樣完整驗證）。
            Err(e)
                if matches!(e.raw_os_error(), Some(80 | 183 | 5))
                    && root.join(&spec.key).exists() =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    })();
    drop(staging_handle);
    // 失敗或被搶先時，只回收本次登錄且身分相符的 staging；未登錄（極早期失敗）則保留。
    let release_staging = || {
        if registered {
            if let Ok(mut registry) = Registry::lock(root)
                && let Some(mut record) = registry.take(&staging_name)
            {
                record.children.clone_from(&created);
                registry.set(record);
                let _ = registry.save();
            }
            let _ = forget_or_remove(root, &staging_name, None);
        }
    };
    match result {
        Ok(true) => Ok(()),
        Ok(false) => {
            release_staging();
            Ok(())
        }
        Err(e) => {
            release_staging();
            Err(io::Error::new(
                e.kind(),
                format!(
                    "無法建立 runtime 快取 {}：{e}",
                    root.join(&spec.key).display()
                ),
            ))
        }
    }
}

// ───────────── 所有權登錄與預設拒絕的回收（RC2 第三輪）─────────────
//
// 回收規則：只處理登錄中、目前 File ID 與登錄相符、實際位置相符、非重新解析點，
// 且沒有任何程序持有租約的目錄。租約＝使用中的執行個體對版本目錄與每個檔案持有的
// 不共享刪除控制代碼（`open_existing`）；回收者必須以 DELETE 權限獨占開啟每一項才會刪除，
// 任一項被持有即整批放棄。只刪除登錄清單內的項目（`cache` 另需 DACL 與本程式設定完全相同）；
// 目錄只在已清空時移除。清單外或不符的內容一律保留。未登錄的目錄完全不碰。

const REGISTRY: &str = "owned.imbc";

#[derive(Clone)]
struct Owned {
    name: String,
    kind: String,
    volume: u64,
    id: [u8; 16],
    /// 實際建立的項目名稱與 File ID。目錄名稱以 '/' 結尾。
    children: Vec<(String, [u8; 16])>,
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

/// 登錄檔以獨占開啟（不共享）作為跨程序鎖；持有期間其他執行個體等待。
struct Registry {
    file: File,
    records: Vec<Owned>,
}
impl Registry {
    fn lock(root: &Path) -> io::Result<Self> {
        let path = root.join(REGISTRY);
        let started = std::time::Instant::now();
        let mut file = loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(&path)
            {
                Ok(file) => break file,
                Err(e)
                    if e.raw_os_error() == Some(32)
                        && started.elapsed() < std::time::Duration::from_secs(60) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("無法鎖定 runtime 快取登錄 {}：{e}", path.display()),
                    ));
                }
            }
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.file_attributes() & REPARSE_ATTRIBUTE != 0
            || link_count(&file)? != 1
        {
            return Err(io::Error::other("runtime 快取登錄不是一般檔案，已拒絕"));
        }
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        // 無法解析的行直接捨棄：對應的目錄因此成為「未登錄」而被完整保留（安全方向）。
        let records = text.lines().filter_map(parse_owned).collect();
        Ok(Self { file, records })
    }
    fn find(&self, name: &str) -> Option<&Owned> {
        self.records
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
    }
    fn take(&mut self, name: &str) -> Option<Owned> {
        let index = self
            .records
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(name))?;
        Some(self.records.remove(index))
    }
    fn set(&mut self, record: Owned) {
        self.take(&record.name);
        self.records.push(record);
    }
    fn save(&mut self) -> io::Result<()> {
        use std::io::Seek;
        let mut text = String::new();
        for r in &self.records {
            let rels = if r.children.is_empty() {
                "-".to_owned()
            } else {
                r.children
                    .iter()
                    .map(|(path, id)| format!("{}:{}", hex(path.as_bytes()), hex(id)))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            text.push_str(&format!(
                "IMBCOWN2 {} {:x} {} {} {rels}\n",
                r.kind,
                r.volume,
                hex(&r.id),
                hex(r.name.as_bytes())
            ));
        }
        self.file.set_len(0)?;
        self.file.seek(io::SeekFrom::Start(0))?;
        self.file.write_all(text.as_bytes())?;
        self.file.sync_all()
    }
}
fn parse_owned(line: &str) -> Option<Owned> {
    let parts: Vec<_> = line.split(' ').collect();
    let [tag, kind, volume, id, name, rels] = parts[..] else {
        return None;
    };
    if !matches!(tag, "IMBCOWN1" | "IMBCOWN2")
        || !matches!(kind, "cache" | "staging" | "quarantine")
    {
        return None;
    }
    let id: [u8; 16] = unhex(id)?.try_into().ok()?;
    let name = String::from_utf8(unhex(name)?).ok()?;
    // 舊版只記錄子項名稱。保留頂層紀錄，但沒有 File ID 的子項不具備刪除授權。
    let children = if tag == "IMBCOWN1" || rels == "-" {
        Vec::new()
    } else {
        rels.split(',')
            .map(|r| {
                let (path, id) = r.split_once(':')?;
                Some((
                    String::from_utf8(unhex(path)?).ok()?,
                    unhex(id)?.try_into().ok()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?
    };
    if !valid_rel(&name)
        || name.contains('/')
        || !children
            .iter()
            .all(|(path, _)| valid_rel(path.trim_end_matches('/')))
    {
        return None;
    }
    Some(Owned {
        name,
        kind: kind.to_owned(),
        volume: u64::from_str_radix(volume, 16).ok()?,
        id,
        children,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Removal {
    /// 已完整移除。
    Gone,
    /// 目錄已不存在。
    Missing,
    /// 有程序持有租約（不共享刪除的控制代碼）：完全未動。
    Leased,
    /// 同名目錄的身分與登錄不符（已被替換）或是重新解析點：完全未動。
    Foreign,
    /// 已刪除清單內且驗證通過的項目；清單外或不符的內容保留。
    Partial,
}

fn open_for_delete(path: &Path, directory: bool) -> io::Result<File> {
    let extra = if directory { LIST | TRAVERSE } else { 0 };
    OpenOptions::new()
        .access_mode(DELETE | READ_CONTROL | READ_ATTRIBUTES | SYNCHRONIZE | extra)
        // 目錄共享讀寫（不共享刪除）、檔案完全不共享：任何其他持有者都會使開啟失敗。
        .share_mode(if directory { 3 } else { 0 })
        .custom_flags(OPEN_REPARSE_POINT | if directory { BACKUP_SEMANTICS } else { 0 })
        .open(path)
}
fn is_plain(file: &File, directory: bool) -> io::Result<bool> {
    let meta = file.metadata()?;
    Ok(meta.file_attributes() & REPARSE_ATTRIBUTE == 0
        && if directory {
            meta.is_dir()
        } else {
            meta.is_file()
        })
}

/// 依登錄安全移除一個目錄。先開啟並驗證全部項目，確認沒有租約後才開始刪除。
fn remove_owned(root: &Path, record: &Owned, file_sd: Option<&Descriptor>) -> io::Result<Removal> {
    let top_path = root.join(&record.name);
    let top = match open_for_delete(&top_path, true) {
        Ok(handle) => handle,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Removal::Missing),
        Err(e) if e.raw_os_error() == Some(32) => return Ok(Removal::Leased),
        Err(e) => return Err(e),
    };
    if !is_plain(&top, true)?
        || super::instance_ledger::file_identity(&top).ok() != Some((record.volume, record.id))
        || norm(&super::filesystem_safety::final_path(&top)?) != norm(&top_path)
    {
        return Ok(Removal::Foreign);
    }
    let mut dirs = vec![top];
    let mut files = Vec::new();
    let mut verified_dirs = BTreeSet::from([""]);
    for (rel, id) in &record.children {
        let directory = rel.ends_with('/');
        let rel = rel.trim_end_matches('/');
        let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
        if !verified_dirs.contains(parent) {
            continue;
        }
        let path = rel_path(&top_path, rel);
        let handle = match open_for_delete(&path, directory) {
            Ok(handle) => handle,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) if e.raw_os_error() == Some(32) => return Ok(Removal::Leased),
            Err(_) => continue, // 無法獨占取得：視為未知內容，保留。
        };
        let verified = is_plain(&handle, directory)?
            && norm(&super::filesystem_safety::final_path(&handle)?) == norm(&path)
            && super::instance_ledger::file_identity(&handle).ok() == Some((record.volume, *id))
            && (directory
                || (link_count(&handle)? == 1
                    && file_sd.map_or(Ok(true), |sd| dacl_matches(&handle, sd))?));
        // 被替換、連結或權限不符：視為未知內容，保留（不加入刪除清單）。
        if verified {
            if directory {
                verified_dirs.insert(rel);
                dirs.push(handle);
            } else {
                files.push(handle);
            }
        }
    }
    // 全部項目都已獨占取得，期間沒有租約：開始刪除。檔案先，目錄由深到淺。
    for file in files {
        let _ = super::controlled_temp::delete_handle(&file);
    }
    while let Some(dir) = dirs.pop() {
        // 非空目錄（含清單外內容）刪除失敗時保留。
        let _ = super::controlled_temp::delete_handle(&dir);
    }
    Ok(match fs::symlink_metadata(&top_path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Removal::Gone,
        _ => Removal::Partial,
    })
}

/// 在登錄鎖內回收一個登錄項目；成功移除或確認不屬於本程式時刪除登錄紀錄。
fn forget_or_remove(root: &Path, name: &str, file_sd: Option<&Descriptor>) -> io::Result<Removal> {
    let mut registry = Registry::lock(root)?;
    let Some(record) = registry.find(name).cloned() else {
        return Ok(Removal::Foreign);
    };
    let sd = if record.kind == "staging" {
        None
    } else {
        file_sd
    };
    let outcome = remove_owned(root, &record, sd)?;
    if matches!(outcome, Removal::Gone | Removal::Missing | Removal::Foreign) {
        registry.take(name);
        registry.save()?;
    }
    Ok(outcome)
}

/// 目前版本的快取驗證失敗：只在能證明由 IMB-C 建立時移除；有清單外內容時，
/// 把殘留目錄改名為 `quarantine-*`（內容完整保留）以便重建。
fn discard(root: &Path, root_handle: &File, name: &str, file_sd: &Descriptor) -> io::Result<()> {
    let mut registry = Registry::lock(root)?;
    let record = registry.find(name).cloned().ok_or_else(|| {
        io::Error::other("無法證明此目錄由 IMB-C 建立，已完整保留；請確認後手動處理")
    })?;
    match remove_owned(root, &record, Some(file_sd))? {
        Removal::Gone | Removal::Missing => {
            registry.take(name);
            registry.save()
        }
        Removal::Leased => Err(io::Error::other("快取正被其他 IMB-C 執行個體使用")),
        Removal::Foreign => Err(io::Error::other(
            "無法證明此目錄由 IMB-C 建立（沒有相符的所有權紀錄），已完整保留；請確認後手動處理",
        )),
        Removal::Partial => {
            let quarantine = random_name("quarantine-")?;
            let handle = open_for_delete(&root.join(name), true)?;
            if !is_plain(&handle, true)?
                || super::instance_ledger::file_identity(&handle)? != (record.volume, record.id)
                || norm(&super::filesystem_safety::final_path(&handle)?) != norm(&root.join(name))
            {
                return Err(io::Error::other("快取身分已改變，已保留"));
            }
            rename_relative(&handle, root_handle, &quarantine)?;
            registry.take(name);
            registry.set(Owned {
                name: quarantine,
                kind: "quarantine".into(),
                ..record
            });
            registry.save()
        }
    }
}

/// 清理其他版本與崩潰留下的 staging：只處理登錄中的項目，其餘（包括未知目錄）完全不碰。
fn cleanup(root: &Path, current: &str, file_sd: &Descriptor) {
    let Ok(registry) = Registry::lock(root) else {
        return;
    };
    let names: Vec<String> = registry
        .records
        .iter()
        .filter(|r| r.kind != "quarantine" && !r.name.eq_ignore_ascii_case(current))
        .map(|r| r.name.clone())
        .collect();
    drop(registry);
    for name in names {
        let _ = forget_or_remove(root, &name, Some(file_sd));
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;
    pub(in super::super) fn sha(path: &Path) -> io::Result<[u8; 32]> {
        let mut hasher = Sha256::new();
        io::copy(&mut File::open(path)?, &mut hasher)?;
        Ok(hasher.finalize().into())
    }
    /// 以指定根目錄、名稱與（可選的指定）雜湊建立／驗證快取。回傳版本目錄與鎖定控制代碼。
    pub(in super::super) fn prepare_at(
        root: &Path,
        key: &str,
        files: &[(&str, &Path, Option<[u8; 32]>)],
    ) -> io::Result<(PathBuf, Vec<File>)> {
        let mut entries = Vec::new();
        for (rel, source, hash) in files {
            entries.push(Entry {
                rel: (*rel).to_owned(),
                hash: match hash {
                    Some(hash) => *hash,
                    None => sha(source)?,
                },
                source: source.to_path_buf(),
            });
        }
        let prepared = prepare(
            root,
            &Spec {
                key: key.to_owned(),
                entries,
            },
        )?;
        Ok((prepared.root.clone(), prepared._pins))
    }
    /// 以正式內嵌清單從 `base`（Portable 目錄）建立快取到 `root`，並註冊給 `resolve` 使用。
    pub(in super::super) fn prepare_production(base: &Path, root: &Path) -> io::Result<PathBuf> {
        let prepared = Arc::new(prepare(
            root,
            &Spec {
                key: production_key(),
                entries: production_entries(base)?,
            },
        )?);
        let path = prepared.root.clone();
        PREPARED
            .lock()
            .map_err(|_| io::Error::other("lock"))?
            .insert(0, prepared);
        Ok(path)
    }
    pub(in super::super) fn production_rels() -> Vec<String> {
        production_entries(Path::new("C:/"))
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect()
    }
    pub(in super::super) fn reset_cleanup(root: &Path) {
        let root =
            super::super::filesystem_safety::final_path(&File::open(root).unwrap_or_else(|_| {
                OpenOptions::new()
                    .access_mode(READ_ATTRIBUTES)
                    .custom_flags(BACKUP_SEMANTICS)
                    .open(root)
                    .unwrap()
            }))
            .unwrap();
        CLEANED.lock().unwrap().remove(&norm(&root));
    }
    /// 從程序內清單移除（測試結束前釋放鎖定，讓暫存目錄可刪除）。
    pub(in super::super) fn unregister(key_dir: &Path) {
        PREPARED
            .lock()
            .unwrap()
            .retain(|p| norm(&p.root) != norm(key_dir));
    }
    pub(in super::super) fn current_user() -> String {
        user_sid().unwrap()
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetNamedSecurityInfoW(
            name: *const u16,
            kind: u32,
            information: u32,
            owner: *mut Handle,
            group: *mut Handle,
            dacl: *mut Handle,
            sacl: *mut Handle,
            descriptor: *mut Handle,
        ) -> u32;
        fn SetNamedSecurityInfoW(
            name: *const u16,
            kind: u32,
            information: u32,
            owner: Handle,
            group: Handle,
            dacl: Handle,
            sacl: Handle,
        ) -> u32;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor: Handle,
            revision: u32,
            information: u32,
            text: *mut *mut u16,
            length: *mut u32,
        ) -> i32;
    }
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    /// 路徑目前 DACL 的 SDDL（只讀取）。
    pub(in super::super) fn sddl_of(path: &Path) -> String {
        let (mut dacl, mut sd) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(
            unsafe {
                GetNamedSecurityInfoW(
                    wide(path).as_ptr(),
                    1,
                    4,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut dacl,
                    std::ptr::null_mut(),
                    &mut sd,
                )
            },
            0
        );
        let sd = Local(sd);
        let mut text = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd.0,
                    1,
                    4,
                    &mut text,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let owner = Local(text.cast());
        let value = unsafe {
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        drop(owner);
        value
    }
    /// 設定 DACL 並讓可繼承 ACE 傳播到既有子項（測試用：模擬唯讀 Portable 目錄）。
    pub(in super::super) fn set_tree_sddl(path: &Path, text: &str) {
        let sd = Descriptor::parse(text).unwrap();
        assert_eq!(
            unsafe {
                SetNamedSecurityInfoW(
                    wide(path).as_ptr(),
                    1,
                    0x8000_0004,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    sd.dacl().unwrap(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
    }
}
