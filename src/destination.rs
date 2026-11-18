//! 以固定的目的目錄控制代碼完成能力檢查、暫存建立與不覆寫發布（RC2 M2）。
//!
//! 目的目錄只以路徑開啟一次；之後的檔案系統判定、剩餘空間、建立暫存（`NtCreateFile` 相對
//! `RootDirectory`）與最終發布（`FileRenameInfo` 相對 `RootDirectory`、`ReplaceIfExists = FALSE`）
//! 都使用同一個控制代碼，不再重新解析路徑。目錄控制代碼不共享刪除，持有期間該目錄無法被
//! 改名或刪除。引擎從不取得目的端任何控制代碼：轉換與驗證都在本機受控暫存完成，
//! 驗證後才由 IMB-C 寫入此處。
use super::filesystem_safety::{self, Protection};
use std::{
    ffi::{OsStr, c_void},
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
};

type Handle = *mut c_void;
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *mut UnicodeString,
    attributes: u32,
    security: Handle,
    quality: Handle,
}
#[repr(C)]
struct IoStatus {
    status: usize,
    information: usize,
}
#[repr(C)]
struct RenameInfo {
    replace: u32,
    root: Handle,
    length: u32,
    name: [u16; 260],
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file: *mut Handle,
        access: u32,
        attributes: *mut ObjectAttributes,
        status: *mut IoStatus,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
    fn NtSetInformationFile(
        file: Handle,
        status: *mut c_void,
        information: *const c_void,
        length: u32,
        class: u32,
    ) -> i32;
}

/// 已開啟並固定的輸出目錄。
pub(super) struct Destination {
    dir: File,
    path: PathBuf,
}
impl Destination {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let dir = OpenOptions::new()
            // LIST | ADD_FILE | TRAVERSE | READ_ATTRIBUTES | SYNCHRONIZE
            .access_mode(0x0010_00a3)
            // 不共享刪除：持有期間目錄無法被改名、刪除或換成其他目錄。
            .share_mode(3)
            .custom_flags(0x0200_0000) // BACKUP_SEMANTICS
            .open(path)
            .map_err(|e| io::Error::new(e.kind(), format!("無法開啟輸出目錄：{e}")))?;
        if !dir.metadata()?.is_dir() {
            return Err(io::Error::other("輸出位置不是目錄"));
        }
        filesystem_safety::check_output(&dir)?;
        let path = filesystem_safety::final_path(&dir)?;
        Ok(Self { dir, path })
    }
    /// 控制代碼目前實際對應的路徑（顯示與跨執行個體預留用；檔案操作不使用它）。
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    #[allow(dead_code)] // 診斷與測試使用
    pub(super) fn protection(&self) -> io::Result<Protection> {
        filesystem_safety::check_output(&self.dir)
    }
    pub(super) fn free_bytes(&self) -> io::Result<u64> {
        filesystem_safety::free_bytes(&self.dir)
    }
    /// 在此目錄下相對建立新的 `.tmp*` 暫存（FILE_CREATE，絕不開啟既有檔案）。
    pub(super) fn create_temp(&self) -> io::Result<OutputTemp> {
        for _ in 0..16 {
            let name = format!(".tmp{}", super::instance_ledger::random_id()?);
            let mut wide: Vec<u16> = OsStr::new(&name).encode_wide().collect();
            let mut unicode = UnicodeString {
                length: (wide.len() * 2) as u16,
                maximum: (wide.len() * 2) as u16,
                buffer: wide.as_mut_ptr(),
            };
            let mut attributes = ObjectAttributes {
                length: std::mem::size_of::<ObjectAttributes>() as u32,
                root: self.dir.as_raw_handle(),
                name: &mut unicode,
                attributes: 0x40, // OBJ_CASE_INSENSITIVE
                security: std::ptr::null_mut(),
                quality: std::ptr::null_mut(),
            };
            let mut status = IoStatus {
                status: 0,
                information: 0,
            };
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                NtCreateFile(
                    &mut handle,
                    // GENERIC_READ | GENERIC_WRITE | DELETE | SYNCHRONIZE
                    0xc011_0000,
                    &mut attributes,
                    &mut status,
                    std::ptr::null(),
                    0x80, // FILE_ATTRIBUTE_NORMAL
                    1,    // 只共享讀取：發布前其他程序無法改寫已驗證的位元組
                    2,    // FILE_CREATE
                    0x60, // FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT
                    std::ptr::null(),
                    0,
                )
            };
            if result == 0xC000_0035u32 as i32 {
                continue; // STATUS_OBJECT_NAME_COLLISION：換名稱，絕不開啟既有檔案
            }
            if result < 0 {
                return Err(io::Error::from_raw_os_error(
                    unsafe { RtlNtStatusToDosError(result) } as i32,
                ));
            }
            let file = unsafe { File::from_raw_handle(handle) };
            return Ok(OutputTemp {
                file,
                path: self.path.join(&name),
                published: false,
            });
        }
        Err(io::Error::other("無法建立唯一的輸出暫存名稱"))
    }
}

/// 目的目錄內的輸出暫存。未發布就釋放時，以自己的控制代碼設定刪除（不以路徑重新開啟）。
pub(super) struct OutputTemp {
    file: File,
    path: PathBuf,
    published: bool,
}
impl OutputTemp {
    pub(super) fn as_file(&self) -> &File {
        &self.file
    }
    pub(super) fn as_file_mut(&mut self) -> &mut File {
        &mut self.file
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    /// 相對同一目錄控制代碼改名為 `name`；目標已存在時回傳 `AlreadyExists`，絕不覆寫。
    pub(super) fn persist_noclobber(
        &mut self,
        destination: &Destination,
        name: &OsStr,
    ) -> io::Result<PathBuf> {
        let wide: Vec<u16> = name.encode_wide().collect();
        if wide.is_empty() || wide.len() >= 260 || wide.iter().any(|c| matches!(c, 0 | 47 | 92)) {
            return Err(io::Error::other("輸出檔名無效"));
        }
        let mut info = RenameInfo {
            replace: 0,
            root: destination.dir.as_raw_handle(),
            length: (wide.len() * 2) as u32,
            name: [0; 260],
        };
        info.name[..wide.len()].copy_from_slice(&wide);
        // FILE_RENAME_INFORMATION（RootDirectory 相對）。SetFileInformationByHandle 對相對
        // RootDirectory 回傳 ERROR_INVALID_PARAMETER（實測），改直接呼叫 NtSetInformationFile。
        let mut status = [0usize; 2];
        let result = unsafe {
            NtSetInformationFile(
                self.file.as_raw_handle(),
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
        self.published = true;
        self.path = destination.path.join(name);
        Ok(self.path.clone())
    }
}
#[cfg(test)]
thread_local! {
    /// 測試：模擬遠端中斷或後端拒絕刪除。
    pub(super) static TEST_FAIL_DISCARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
impl OutputTemp {
    /// 放棄未發布的暫存：以自己的控制代碼設定刪除並關閉，**回報**失敗（SMB 中斷、後端拒絕等），
    /// 讓呼叫端保留擁有權紀錄供下次啟動以 File ID 安全回收，而不是默默忽略（RC2 第三輪）。
    pub(super) fn discard(mut self) -> io::Result<()> {
        // 不論結果，Drop 都不再重試：失敗時檔案保留給紀錄回收。
        self.published = true;
        #[cfg(test)]
        if TEST_FAIL_DISCARD.with(|v| v.get()) {
            return Err(io::Error::other("injected delete failure"));
        }
        super::controlled_temp::delete_handle(&self.file)?;
        Ok(())
    }
}
impl Drop for OutputTemp {
    fn drop(&mut self) {
        // 只作為 panic 等非預期路徑的最後防線；正常失敗路徑一律經 `discard` 回報結果。
        if !self.published {
            let _ = super::controlled_temp::delete_handle(&self.file);
        }
    }
}
