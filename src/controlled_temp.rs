//! 僅清理本次建立的目錄；保留祖先控制代碼，阻止工作期間重新命名或替換。
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

fn pin(path: &Path, sharing: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(0xa0) // READ_ATTRIBUTES | TRAVERSE
        .share_mode(sharing)
        .custom_flags(0x02200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other("暫存路徑包含重新解析點或非目錄，已拒絕"));
    }
    Ok(file)
}

/// 鎖定 `path` 及其所有祖先目錄（不共享 DELETE），持有期間無法被改名或替換成連結。
pub(super) fn pin_ancestors(path: &Path) -> io::Result<Vec<File>> {
    let mut pins = Vec::new();
    let mut current = PathBuf::new();
    for c in path.components() {
        current.push(c.as_os_str());
        if matches!(c, Component::RootDir | Component::Normal(_)) {
            pins.push(pin(&current, 3)?);
        }
    }
    Ok(pins)
}

/// 以既有控制代碼設定刪除處置，不重新以路徑開啟。
pub(super) fn delete_handle(file: &File) -> io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let delete: u8 = 1; // FILE_DISPOSITION_INFO::DeleteFile (BOOLEAN)
    if unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u8).cast(), 1)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) struct ControlledDir {
    path: PathBuf,
    own: Option<File>,
    _ancestors: Vec<File>,
    retained: AtomicBool,
    // 根層級目錄的擁有權日誌項目；確認清理後才釋放，保留時留給崩潰回收。
    entry: Option<super::instance_ledger::Entry>,
}
impl ControlledDir {
    pub(super) fn allow_sandbox(&self, sandbox: &super::low_integrity::Sandbox) -> io::Result<()> {
        sandbox.grant_directory(
            self.own
                .as_ref()
                .ok_or_else(|| io::Error::other("暫存目錄已關閉"))?,
        )
    }
    pub(super) fn child(&self, prefix: &str) -> io::Result<Self> {
        let mut ancestors = self
            ._ancestors
            .iter()
            .map(File::try_clone)
            .collect::<io::Result<Vec<_>>>()?;
        ancestors.push(
            self.own
                .as_ref()
                .ok_or_else(|| io::Error::other("暫存目錄已關閉"))?
                .try_clone()?,
        );
        Self::with_pins(&self.path, prefix, ancestors)
    }
    pub(super) fn create(root: &Path, prefix: &str) -> io::Result<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(io::Error::other("暫存根目錄必須為不含路徑穿越的絕對路徑"));
        }
        let mut ancestors = Vec::new();
        let mut path = PathBuf::new();
        for c in root.components() {
            path.push(c.as_os_str());
            if matches!(c, Component::RootDir | Component::Normal(_)) {
                ancestors.push(pin(&path, 1)?);
            }
        }
        require_ntfs(root)?;
        // 執行個體紀錄先建立；無法記錄擁有權就不建立目錄，避免產生無法安全回收的殘留。
        let ledger = super::instance_ledger::global()?;
        let mut owned = Self::with_pins(root, prefix, ancestors)?;
        // 建立與記錄之間崩潰會留下未記錄的空目錄（名稱隨機、無內容）；只會漏收，不會誤刪。
        owned.entry = Some(ledger.record(
            super::instance_ledger::Kind::Dir,
            owned.own.as_ref().unwrap(),
            &owned.path,
        )?);
        // 目錄建立前不共享 WRITE；建立後每個祖先都有不可移除的下一層目錄。
        // NTFS 不允許在非空目錄設定 reparse point，因此可允許祖先 WRITE sharing，
        // 供同一祖先內的正常輸出 rename 使用。仍不共享 DELETE；新工作目錄本身維持 READ only sharing。
        let mut relaxed = Vec::new();
        let mut path = PathBuf::new();
        for c in root.components() {
            path.push(c.as_os_str());
            if matches!(c, Component::RootDir | Component::Normal(_)) {
                relaxed.push(pin(&path, 3)?);
            }
        }
        owned._ancestors = relaxed;
        Ok(owned)
    }
    fn with_pins(root: &Path, prefix: &str, ancestors: Vec<File>) -> io::Result<Self> {
        let (name, own) = create_directory(
            ancestors
                .last()
                .ok_or_else(|| io::Error::other("暫存父目錄缺少控制代碼"))?,
            prefix,
        )?;
        let path = root.join(name);
        Ok(Self {
            path,
            own: Some(own),
            _ancestors: ancestors,
            retained: AtomicBool::new(false),
            entry: None,
        })
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn retain(&self) {
        self.retained.store(true, Ordering::Relaxed);
    }
    pub(super) fn keep(self) -> PathBuf {
        self.retain();
        self.path.clone()
    }
    fn cleanup(&mut self) -> io::Result<()> {
        if std::thread::panicking() {
            self.retain();
        }
        if self.retained.load(Ordering::Relaxed) || self.own.is_none() {
            return Ok(());
        }
        let own = self.own.as_ref().unwrap();
        remove_children(&self.path, own)?;
        if let Err(error) = delete_handle(own) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "以原始控制代碼移除目錄失敗 {}：{}",
                    self.path.display(),
                    error
                ),
            ));
        }
        self.own.take(); // 以原始控制代碼刪除根目錄，沒有重新開啟路徑的空窗。
        if let Some(entry) = self.entry.take() {
            entry.release();
        }
        Ok(())
    }
    pub(super) fn close(mut self) -> io::Result<()> {
        let result = self.cleanup();
        self.retain(); // 失敗不盲目重試刪除。
        result
    }
}

/// 刪除 `path`（由 `dir` 控制代碼持有）內的所有項目，不跟隨連結。
/// 項目若已是 delete-pending（其他程序仍持有已刪除檔案的控制代碼），以 NT 狀態碼精確辨識後
/// 有界等待其消失；其他 AccessDenied 仍照原樣失敗，不放寬權限判斷。
pub(super) fn remove_children(path: &Path, dir: &File) -> io::Result<()> {
    let mut pending = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("清理目錄列舉失敗 {}：{e}", path.display()),
        )
    })? {
        let entry = entry.map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("清理目錄列舉失敗 {}：{e}", path.display()),
            )
        })?;
        let child = entry.path();
        let name = entry.file_name();
        // Rust Windows remove_dir_all 使用 no-follow／handle-relative 遍歷。
        // 自身及祖先保持鎖定；不跨越子目錄的符號連結／junction。
        let meta = match fs::symlink_metadata(&child) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => match after_failure(dir, &name)? {
                // 失敗後名稱已消失：delete-pending 的持有者剛好釋放，刪除已完成。
                EntryState::Gone => continue,
                EntryState::DeletePending => {
                    pending.push((name, child));
                    continue;
                }
                _ => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("清理項目中繼資料讀取失敗 {}：{e}", child.display()),
                    ));
                }
            },
        };
        let removed = if meta.file_attributes() & 0x410 == 0x410 {
            fs::remove_dir(&child)
        } else if meta.is_dir() {
            fs::remove_dir_all(&child)
        } else {
            fs::remove_file(&child)
        };
        match removed {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => match after_failure(dir, &name)? {
                EntryState::Gone => (),
                EntryState::DeletePending => pending.push((name, child)),
                _ => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("清理項目移除失敗 {}：{e}", child.display()),
                    ));
                }
            },
        }
    }
    wait_delete_pending(dir, pending, delete_pending_wait())
}

/// 以路徑操作失敗後的實際狀態。只有「已消失」與「已標記刪除」視為刪除中；
/// 項目仍正常存在（真正的拒絕或仍在使用）一律照原錯誤失敗。
fn after_failure(dir: &File, name: &std::ffi::OsStr) -> io::Result<EntryState> {
    #[cfg(test)]
    if let Some(hook) = TEST_BEFORE_PROBE.with(|v| v.borrow_mut().take()) {
        hook();
    }
    entry_state(dir, name)
}

fn delete_pending_wait() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = TEST_PENDING_WAIT_MS.with(|v| v.get()) {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(10)
}

fn wait_delete_pending(
    dir: &File,
    pending: Vec<(std::ffi::OsString, PathBuf)>,
    limit: std::time::Duration,
) -> io::Result<()> {
    let started = std::time::Instant::now();
    let mut remaining = pending;
    while !remaining.is_empty() {
        let mut still = Vec::new();
        for (name, child) in remaining {
            match entry_state(dir, &name)? {
                EntryState::Gone => (),
                EntryState::DeletePending => still.push((name, child)),
                state => {
                    // 刪除完成後同名項目又出現，或狀態無法判讀：不猜測，保留目錄。
                    return Err(io::Error::other(format!(
                        "清理項目狀態改變（{state:?}），已保留目錄 {}",
                        child.display()
                    )));
                }
            }
        }
        remaining = still;
        if remaining.is_empty() {
            break;
        }
        if started.elapsed() >= limit {
            let names: Vec<_> = remaining
                .iter()
                .map(|(_, p)| p.display().to_string())
                .collect();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "清理項目已標記刪除但仍被其他程序持有，等待 {} 秒後保留目錄：{}",
                    limit.as_secs_f32(),
                    names.join("、")
                ),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum EntryState {
    Gone,
    DeletePending,
    Present,
    Other(i32),
}

/// 以父目錄控制代碼相對開啟單一名稱，取得未經 Win32 對應的 NT 狀態。
/// STATUS_DELETE_PENDING 與真正的存取拒絕在 Win32 都是錯誤 5，只能在這層區分。
pub(super) fn entry_state(dir: &File, name: &std::ffi::OsStr) -> io::Result<EntryState> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            handle: *mut *mut std::ffi::c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatus,
            allocation: *const i64,
            file_attributes: u32,
            sharing: u32,
            disposition: u32,
            options: u32,
            ea: *const std::ffi::c_void,
            ea_size: u32,
        ) -> i32;
        fn NtClose(handle: *mut std::ffi::c_void) -> i32;
    }
    let mut wide: Vec<u16> = name.encode_wide().collect();
    if wide.is_empty()
        || wide.len() > 255
        || wide.iter().any(|c| *c == b'\\' as u16 || *c == b'/' as u16)
    {
        return Err(io::Error::other("清理項目名稱無效"));
    }
    let mut unicode = UnicodeString {
        length: (wide.len() * 2) as u16,
        maximum: (wide.len() * 2) as u16,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root: dir.as_raw_handle(),
        name: &mut unicode,
        attributes: 0x1040, // CASE_INSENSITIVE | DONT_REPARSE
        security: std::ptr::null_mut(),
        qos: std::ptr::null_mut(),
    };
    let mut handle = std::ptr::null_mut();
    let mut iosb = IoStatus::default();
    // FILE_READ_ATTRIBUTES | SYNCHRONIZE；共享全開，只觀察不阻擋其他持有者。
    // FILE_OPEN；OPEN_REPARSE_POINT | OPEN_FOR_BACKUP_INTENT | SYNCHRONOUS_IO_NONALERT。
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            0x100080,
            &mut attributes,
            &mut iosb,
            std::ptr::null(),
            0,
            7,
            1,
            0x204020,
            std::ptr::null(),
            0,
        )
    };
    Ok(match status as u32 {
        0 => {
            unsafe { NtClose(handle) };
            EntryState::Present
        }
        0xC0000056 => EntryState::DeletePending,
        0xC0000034 => EntryState::Gone, // OBJECT_NAME_NOT_FOUND
        _ => EntryState::Other(status),
    })
}

fn require_ntfs(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if TEST_REJECT_FILESYSTEM.with(|v| v.get()) {
        return Err(io::Error::other("fixture filesystem rejected"));
    }
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, size: u32) -> i32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_size: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            system: *mut u16,
            system_size: u32,
        ) -> i32;
    }
    let path = path.canonicalize()?;
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = [0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut system = [0u16; 32];
    let mut flags = 0;
    if unsafe {
        GetVolumeInformationW(
            mount.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            system.as_mut_ptr(),
            system.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let name = String::from_utf16_lossy(
        &system[..system.iter().position(|n| *n == 0).unwrap_or(system.len())],
    );
    if !name.eq_ignore_ascii_case("NTFS") || flags & 8 == 0 {
        return Err(io::Error::other(
            "受控暫存需要支援 ACL 的 NTFS 目錄；請設定可用的 MAGICK_TEMPORARY_PATH，不改用 C 槽",
        ));
    }
    Ok(())
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: *mut std::ffi::c_void,
    name: *mut UnicodeString,
    attributes: u32,
    security: *mut std::ffi::c_void,
    qos: *mut std::ffi::c_void,
}
#[repr(C)]
#[derive(Default)]
struct IoStatus {
    status: usize,
    information: usize,
}
fn create_directory(parent: &File, prefix: &str) -> io::Result<(String, File)> {
    use std::os::windows::io::FromRawHandle;
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            bytes: *mut u8,
            len: u32,
            flags: u32,
        ) -> i32;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            handle: *mut *mut std::ffi::c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatus,
            allocation: *const i64,
            file_attributes: u32,
            sharing: u32,
            disposition: u32,
            options: u32,
            ea: *const std::ffi::c_void,
            ea_size: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(io::Error::other("不合法的暫存名稱字首"));
    }
    let mut random = [0u8; 16];
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), random.as_mut_ptr(), 16, 2) };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    let name = format!(
        "{prefix}{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let mut wide: Vec<_> = name.encode_utf16().collect();
    let mut unicode = UnicodeString {
        length: (wide.len() * 2) as u16,
        maximum: (wide.len() * 2) as u16,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root: parent.as_raw_handle(),
        name: &mut unicode,
        attributes: 0x1040, // CASE_INSENSITIVE | DONT_REPARSE
        security: std::ptr::null_mut(),
        qos: std::ptr::null_mut(),
    };
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut std::ffi::c_void,
            size: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    // 保護 DACL；僅目錄擁有者與 SYSTEM 可完整存取，子檔案／子目錄繼承。
    // OW 是 Windows Owner Rights SID，不依賴可偽造的使用者名稱環境變數。
    let sddl: Vec<_> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    attributes.security = descriptor;
    let mut handle = std::ptr::null_mut();
    let mut iosb = IoStatus::default();
    // FILE_CREATE：只建立新目錄，原子取得不允許 WRITE／DELETE sharing 的控制代碼。
    // 不使用 CreateDirectory → 重新開啟的兩步流程，避免建立後被替換。
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            0x1f00a1, // 原有權限 + READ_CONTROL／WRITE_DAC／WRITE_OWNER，僅供新建目錄的隔離授權
            &mut attributes,
            &mut iosb,
            std::ptr::null(),
            0x10,
            1,
            2,
            0x21,
            std::ptr::null(),
            0,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    let file = unsafe { File::from_raw_handle(handle) };
    if iosb.information != 2 {
        return Err(io::Error::other("暫存目錄不是本次新建，已拒絕清理"));
    }
    Ok((name, file))
}

#[cfg(test)]
thread_local! { static TEST_REJECT_FILESYSTEM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
thread_local! { pub(super) static TEST_PENDING_WAIT_MS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
// 在「路徑操作失敗之後、狀態查詢之前」執行，重現持有者剛好釋放的競態（實際引擎測試曾觀察到）。
#[cfg(test)]
thread_local! { static TEST_BEFORE_PROBE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn filesystem_rejection_precedes_directory_creation() {
        let root = tempfile::tempdir_in(preferred_root().unwrap()).unwrap();
        TEST_REJECT_FILESYSTEM.with(|v| v.set(true));
        let result = ControlledDir::create(root.path(), "imb-c-rejected-");
        TEST_REJECT_FILESYSTEM.with(|v| v.set(false));
        assert!(result.is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn unwind_does_not_delete_owned_directory() {
        let mut path = PathBuf::new();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dir = ControlledDir::create(&preferred_root().unwrap(), "imb-c-panic-").unwrap();
            path = dir.path().to_owned();
            fs::write(path.join("sentinel"), b"preserved").unwrap();
            panic!("controlled directory fixture unwind");
        }));
        assert!(panic.is_err());
        assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"preserved");
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn native_observation_handles_delete_pending_without_reopening() {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn DeleteFileW(path: *const u16) -> i32;
        }
        let root = tempfile::tempdir_in(preferred_root().unwrap()).unwrap();
        let path = root.path().join("native-cache");
        let mut hold = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(7)
            .open(&path)
            .unwrap();
        hold.write_all(&[42; 1024]).unwrap();
        hold.sync_all().unwrap();
        let entry = fs::read_dir(root.path()).unwrap().next().unwrap().unwrap();
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(unsafe { DeleteFileW(wide.as_ptr()) }, 0);
        let error = fs::symlink_metadata(&path).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(2 | 5)), "{error}");
        // 同一次列舉的快照仍可計數；不以略過 AccessDenied 的方式放過權限問題。
        assert_eq!(native_entry_size(entry).unwrap(), 1024);
        drop(hold);
        assert!(!path.exists());
    }
    #[test]
    fn native_observation_rejects_directories_and_reparse_points() {
        let root = tempfile::tempdir_in(preferred_root().unwrap()).unwrap();
        let target = tempfile::tempdir_in(preferred_root().unwrap()).unwrap();
        let child = root.path().join("unexpected-directory");
        fs::create_dir(&child).unwrap();
        assert!(native_size(root.path()).is_err());
        fs::remove_dir(child).unwrap();
        let sentinel = target.path().join("keep");
        fs::write(&sentinel, b"original").unwrap();
        let link = root.path().join("external-link");
        std::os::windows::fs::symlink_dir(target.path(), link).unwrap();
        assert!(native_size(root.path()).is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"original");
    }
    // UNC 間歇清理失敗的本機重現：匿名暫存（DELETE_ON_CLOSE，與 tempfile::tempfile_in 相同）
    // 關閉時若仍有第三方控制代碼，名稱會以 delete-pending 留在目錄；以路徑開啟或刪除會得到
    // STATUS_DELETE_PENDING → Win32 錯誤 5。修正前 close() 以「清理項目移除失敗…存取被拒」失敗。
    fn pending_entry(dir: &ControlledDir) -> (PathBuf, File) {
        let path = dir.path().join("anonymous-like");
        let first = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(7)
            .custom_flags(0x04000000)
            .open(&path)
            .unwrap();
        let holder = File::options()
            .read(true)
            .share_mode(7)
            .open(&path)
            .unwrap();
        drop(first);
        assert_eq!(File::open(&path).unwrap_err().raw_os_error(), Some(5));
        assert_eq!(
            entry_state(
                dir.own.as_ref().unwrap(),
                std::ffi::OsStr::new("anonymous-like")
            )
            .unwrap(),
            EntryState::DeletePending
        );
        (path, holder)
    }
    #[test]
    fn delete_pending_entry_is_waited_for_then_directory_is_removed() {
        let dir = ControlledDir::create(&preferred_root().unwrap(), "imb-c-pending-").unwrap();
        let root = dir.path().to_owned();
        let (path, holder) = pending_entry(&dir);
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(holder);
        });
        dir.close().unwrap();
        release.join().unwrap();
        assert!(!path.exists() && !root.exists());
    }
    // 實際引擎測試（rukamizugi.psd）觀察到：路徑操作得到錯誤 5 後、狀態查詢前持有者已釋放，
    // 名稱消失。修正前被誤判為失敗並保留空目錄；現在視為刪除已完成。
    #[test]
    fn delete_pending_released_between_failure_and_probe_is_success() {
        let dir = ControlledDir::create(&preferred_root().unwrap(), "imb-c-pending-").unwrap();
        let root = dir.path().to_owned();
        let (path, holder) = pending_entry(&dir);
        TEST_BEFORE_PROBE.with(|v| *v.borrow_mut() = Some(Box::new(move || drop(holder))));
        dir.close().unwrap();
        assert!(
            TEST_BEFORE_PROBE.with(|v| v.borrow().is_none()),
            "競態路徑未被執行"
        );
        assert!(!path.exists() && !root.exists());
    }
    #[test]
    fn delete_pending_beyond_limit_retains_directory() {
        let dir = ControlledDir::create(&preferred_root().unwrap(), "imb-c-pending-").unwrap();
        let root = dir.path().to_owned();
        fs::write(root.join("sentinel"), b"kept").unwrap();
        let (_path, holder) = pending_entry(&dir);
        TEST_PENDING_WAIT_MS.with(|v| v.set(Some(200)));
        let error = dir.close().unwrap_err();
        TEST_PENDING_WAIT_MS.with(|v| v.set(None));
        assert!(error.to_string().contains("仍被其他程序持有"), "{error}");
        assert!(root.exists());
        drop(holder);
        // 本測試建立的目錄；holder 釋放後安全移除。
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn live_holder_without_delete_sharing_is_not_treated_as_delete_pending() {
        let dir = ControlledDir::create(&preferred_root().unwrap(), "imb-c-locked-").unwrap();
        let root = dir.path().to_owned();
        let file = root.join("locked");
        fs::write(&file, b"x").unwrap();
        // 仍在使用、不允許刪除的項目：不是 delete-pending，必須照原樣失敗並保留。
        let holder = File::options()
            .read(true)
            .share_mode(1)
            .open(&file)
            .unwrap();
        assert_ne!(
            entry_state(dir.own.as_ref().unwrap(), std::ffi::OsStr::new("locked")).unwrap(),
            EntryState::DeletePending
        );
        let error = dir.close().unwrap_err();
        assert!(error.to_string().contains("清理項目移除失敗"), "{error}");
        assert!(file.exists());
        drop(holder);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_sdk_layouts() {
        assert_eq!(std::mem::size_of::<super::UnicodeString>(), 16);
        assert_eq!(std::mem::size_of::<super::ObjectAttributes>(), 48);
        assert_eq!(std::mem::size_of::<super::IoStatus>(), 16);
    }
}
impl Drop for ControlledDir {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub(super) const DEFAULT_ROOT: &str = r"D:\ImageMagick-Temp";

pub(super) fn preferred_root() -> io::Result<PathBuf> {
    let root = std::env::var_os("MAGICK_TEMPORARY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    if !root.is_absolute() || !root.is_dir() {
        return Err(io::Error::other(
            "ImageMagick 暫存根目錄不存在或不是絕對路徑；請設定 MAGICK_TEMPORARY_PATH 或建立 D:\\ImageMagick-Temp；不自動改用 C 槽",
        ));
    }
    Ok(root)
}

/// UNC 路徑、網路磁碟機，或解析後指向網路的符號連結。
/// Low IL 引擎子程序無法存取這些位置（實測 `\\localhost\D$` 回報路徑無效）。
pub(super) fn is_network_path(path: &Path) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Prefix;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, size: u32) -> i32;
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    const DRIVE_REMOTE: u32 = 4;
    let unc = |p: &Path| {
        matches!(
            p.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
        )
    };
    if unc(path) {
        return Ok(true);
    }
    let canonical = path.canonicalize()?;
    if unc(&canonical) {
        return Ok(true);
    }
    let wide: Vec<_> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = [0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { GetDriveTypeW(mount.as_ptr()) } == DRIVE_REMOTE)
}

/// 引擎暫存一律放在本機 NTFS。要求的根目錄是網路路徑時，依序改用
/// `MAGICK_TEMPORARY_PATH`（若為本機）與 `D:\ImageMagick-Temp`；都不可用就回報錯誤，
/// 不改用 C 槽或系統 TEMP。第二個回傳值是被取代的原要求路徑，呼叫端必須告知使用者。
pub(super) fn local_engine_root(requested: &Path) -> io::Result<(PathBuf, Option<PathBuf>)> {
    if !is_network_path(requested)? {
        return Ok((requested.to_path_buf(), None));
    }
    let mut candidates = Vec::new();
    if let Ok(root) = preferred_root() {
        candidates.push(root);
    }
    candidates.push(PathBuf::from(DEFAULT_ROOT));
    for candidate in candidates {
        if candidate.is_absolute()
            && candidate.is_dir()
            && matches!(is_network_path(&candidate), Ok(false))
            && require_ntfs(&candidate).is_ok()
        {
            return Ok((candidate, Some(requested.to_path_buf())));
        }
    }
    Err(io::Error::other(format!(
        "暫存根目錄 {} 是網路路徑，Low IL 引擎無法存取；請將 MAGICK_TEMPORARY_PATH 設為本機 NTFS 目錄或建立 D:\\ImageMagick-Temp；不自動改用 C 槽",
        super::display_path(requested)
    )))
}

pub(super) fn free_bytes(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            free: *mut u64,
            total: *mut u64,
            all_free: *mut u64,
        ) -> i32;
    }
    let canonical = path.canonicalize()?;
    let wide: Vec<_> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0;
    if unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}

pub(super) fn check_free(path: &Path, needed: u64) -> io::Result<()> {
    let free = free_bytes(path)?;
    if free < needed {
        return Err(io::Error::other(format!(
            "磁碟空間不足：{} 可用 {} MiB，需要預估 {} MiB（含安全餘裕；不是硬配額）",
            path.display(),
            free / 1048576,
            needed / 1048576
        )));
    }
    Ok(())
}

pub(super) fn volume(path: &Path) -> io::Result<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, size: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(mount: *const u16, name: *mut u16, size: u32) -> i32;
    }
    let canonical = path.canonicalize()?;
    let path: Vec<_> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = [0u16; 32768];
    if unsafe { GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name = [0u16; 1024];
    let value = if unsafe {
        GetVolumeNameForVolumeMountPointW(mount.as_ptr(), name.as_mut_ptr(), name.len() as u32)
    } != 0
    {
        &name[..]
    } else {
        &mount[..]
    }; // SMB 使用 share mount；不把不同 share 當成同一磁碟。
    Ok(String::from_utf16_lossy(
        &value[..value.iter().position(|n| *n == 0).unwrap_or(value.len())],
    )
    .to_ascii_lowercase())
}

pub(super) fn preflight(
    temp: &Path,
    output: &Path,
    pixels: u64,
    source_bytes: u64,
) -> io::Result<Vec<super::instance_ledger::Reservation>> {
    let gib = 1024 * 1024 * 1024;
    // Q16-HDRI 像素快取、MIFF／驗證資料、原生暫存及受控輸出的保守需求估算。
    // 2026-11-13：改為跨執行個體預留。同一磁碟區上所有存活工作（含其他 IMB-C 程序）
    // 的預估量一併扣除，不再重複使用同一份可用空間。這仍是協調紀錄，不是檔案系統硬配額。
    let temporary = 8 * gib + pixels.saturating_mul(48) + source_bytes;
    let destination = pixels.saturating_mul(8) + 64 * 1024 * 1024;
    let ledger = super::instance_ledger::global()?;
    let (temp_volume, output_volume) = (volume(temp)?, volume(output)?);
    if temp_volume == output_volume {
        Ok(vec![reserve_disk(
            &ledger,
            temp,
            &temp_volume,
            temporary.saturating_add(destination),
        )?])
    } else {
        let first = reserve_disk(&ledger, temp, &temp_volume, temporary)?;
        let second = reserve_disk(&ledger, output, &output_volume, destination)?;
        Ok(vec![first, second])
    }
}

fn reserve_disk(
    ledger: &std::sync::Arc<super::instance_ledger::Ledger>,
    path: &Path,
    volume: &str,
    needed: u64,
) -> io::Result<super::instance_ledger::Reservation> {
    let margin = 2 * 1024 * 1024 * 1024;
    let free = free_bytes(path)?;
    match ledger.try_reserve(
        super::instance_ledger::Resource::Disk(volume),
        needed,
        free.saturating_sub(margin),
    )? {
        Ok(reservation) => Ok(reservation),
        Err(held) => Err(io::Error::other(format!(
            "磁碟空間不足：{} 可用 {} MiB，其他進行中工作（含其他 IMB-C 執行個體）已預留 {} MiB，本工作預估需要 {} MiB 另加 2 GiB 安全餘裕（預留是協調紀錄，不是硬配額）",
            path.display(),
            free / 1048576,
            held / 1048576,
            needed / 1048576
        ))),
    }
}

pub(super) fn native_size(path: &Path) -> io::Result<u64> {
    let mut size = 0u64;
    for (i, entry) in fs::read_dir(path)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("原生暫存目錄列舉失敗 {}：{e}", path.display()),
            )
        })?
        .enumerate()
    {
        if i >= 10000 {
            return Err(io::Error::other("原生暫存項目過多"));
        }
        size = size
            .checked_add(native_entry_size(entry?)?)
            .ok_or_else(|| io::Error::other("暫存大小溢位"))?;
    }
    Ok(size)
}

fn native_entry_size(entry: fs::DirEntry) -> io::Result<u64> {
    let path = entry.path();
    // Windows 的 DirEntry.metadata 使用同一次列舉的 no-follow 快照，
    // 不重新開啟可能已被引擎標記刪除的檔名。此處僅觀察用量，
    // 不作清理授權或硬配額；快照與輪詢仍可能落後實際寫入。
    let metadata = match entry.metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("原生暫存中繼資料讀取失敗 {}：{e}", path.display()),
            ));
        }
    };
    if metadata.file_attributes() & 0x400 != 0 || !metadata.is_file() {
        return Err(io::Error::other(
            "原生暫存出現非一般檔案，已停止工作並保留安全清理邊界",
        ));
    }
    Ok(metadata.len())
}
