//! 以實際已開啟的控制代碼判斷工作路徑的安全能力（RC2 H3）。
//!
//! 遠端與否、檔案系統名稱與 ACL 能力都取自**同一個**已開啟的控制代碼
//! （`NtQueryVolumeInformationFile` 的裝置與屬性資訊），不再以路徑重新解析磁碟區，
//! 因此路徑在檢查後被改指向別處也不會讓兩者描述不同物件。
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};

#[repr(C)]
struct IoStatus {
    status: isize,
    information: usize,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryVolumeInformationFile(
        file: *mut c_void,
        status: *mut IoStatus,
        information: *mut c_void,
        length: u32,
        class: u32,
    ) -> i32;
}

/// 路徑所在磁碟區對隔離引擎的保護方式。
///
/// 引擎在每工作獨立的 AppContainer 內執行：未授權的檔案、無 ACL 的卸除式 FAT／exFAT
/// 磁碟區與 SMB 共用都無法以路徑開啟（2026-11-18 以 G: exFAT 與 `\\localhost\C$` 實測）。
/// 引擎只透過繼承的唯讀 stdin 控制代碼讀取來源；為了不把指向未受 NTFS ACL 保護
/// 檔案的控制代碼交給引擎，這類來源先由 IMB-C 複製到本機 NTFS 受控暫存再解碼。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Protection {
    /// 本機 NTFS：來源檔 DACL 未授權 AppContainer，可直接以唯讀控制代碼交給引擎。
    LocalNtfs,
    /// 無永久 ACL（FAT／exFAT）、其他 ACL 檔案系統（ReFS 等未實機驗證）或遠端共用：
    /// 來源先複製到受控暫存。值為顯示用的檔案系統描述。
    CopyFirst(String),
}
fn classify(filesystem: &str, flags: u32, remote: bool, known: bool) -> io::Result<Protection> {
    let name = if filesystem.is_empty() {
        "未知".to_owned()
    } else {
        filesystem.to_owned()
    };
    // 遠端一律複製（從已開啟的控制代碼讀取），即使查不到檔案系統名稱也不會比複製更寬鬆。
    if remote {
        return Ok(Protection::CopyFirst(format!("遠端 {name}")));
    }
    // 本機但無法取得能力：無法判斷是否為可信的本機磁碟區，維持拒絕。
    if !known {
        return Err(io::Error::other("無法確認本機檔案系統安全能力，已拒絕轉檔"));
    }
    if flags & 8 == 0 || !filesystem.eq_ignore_ascii_case("NTFS") {
        return Ok(Protection::CopyFirst(name));
    }
    Ok(Protection::LocalNtfs)
}
#[cfg(test)]
thread_local! { pub(super) static TEST_CAPABILITIES: std::cell::RefCell<Option<(&'static str,u32,bool,bool)>> = const { std::cell::RefCell::new(None) }; }

/// 由控制代碼讀取（檔案系統名稱、屬性旗標、是否遠端、名稱與旗標是否取得）。
/// 裝置資訊查不到時直接拒絕：無法確認是否遠端，就無法決定安全處理方式。
fn query(file: &File) -> io::Result<(String, u32, bool, bool)> {
    let handle = file.as_raw_handle();
    let mut status = IoStatus {
        status: 0,
        information: 0,
    };
    // FileFsDeviceInformation：{ DeviceType, Characteristics }
    let mut device = [0u32; 2];
    let result = unsafe {
        NtQueryVolumeInformationFile(handle, &mut status, device.as_mut_ptr().cast(), 8, 4)
    };
    if result < 0 {
        return Err(io::Error::other(format!(
            "無法由控制代碼確認磁碟區類型（NTSTATUS {result:#010x}），已拒絕轉檔"
        )));
    }
    // FILE_REMOTE_DEVICE 或 FILE_DEVICE_NETWORK_FILE_SYSTEM。
    let remote = device[1] & 0x10 != 0 || device[0] == 0x14;
    // FileFsAttributeInformation：{ Attributes, MaxComponentLength, NameLength, Name[] }
    let mut attributes = [0u32; 130];
    let result = unsafe {
        NtQueryVolumeInformationFile(
            handle,
            &mut status,
            attributes.as_mut_ptr().cast(),
            std::mem::size_of_val(&attributes) as u32,
            5,
        )
    };
    if result < 0 {
        return Ok((String::new(), 0, remote, false));
    }
    let length = (attributes[2] as usize / 2).min((attributes.len() - 3) * 2);
    let name: Vec<u16> = attributes[3..]
        .iter()
        .flat_map(|w| [*w as u16, (*w >> 16) as u16])
        .take(length)
        .collect();
    Ok((String::from_utf16_lossy(&name), attributes[0], remote, true))
}

/// 查詢只讀取中繼資料，不寫入來源、磁碟或磁碟區標籤。
pub(super) fn check(file: &File) -> io::Result<Protection> {
    #[cfg(test)]
    if let Some((fs, flags, remote, known)) = TEST_CAPABILITIES.with(|v| *v.borrow()) {
        return classify(fs, flags, remote, known);
    }
    let (fs, flags, remote, known) = query(file)?;
    classify(&fs, flags, remote, known)
}
/// runtime 快取位置：一律以實際查詢判定（不受測試能力注入影響）。
pub(super) fn check_actual(file: &File) -> io::Result<Protection> {
    let (fs, flags, remote, known) = query(file)?;
    classify(&fs, flags, remote, known)
}
/// 輸出目錄：引擎不接觸目的端（只寫管線，驗證使用本機受控副本），輸出由 IMB-C 以目錄控制代碼
/// 相對建立、寫入與發布，因此 FAT／exFAT／SMB 都可使用；但檔案系統能力必須能由控制代碼確認，
/// 遠端查不到能力也拒絕。
pub(super) fn check_output(file: &File) -> io::Result<Protection> {
    #[cfg(test)]
    if let Some((fs, flags, remote, known)) = TEST_CAPABILITIES.with(|v| *v.borrow()) {
        if !known {
            return Err(io::Error::other(
                "無法確認輸出位置的檔案系統能力，已拒絕轉檔",
            ));
        }
        return classify(fs, flags, remote, known);
    }
    let (fs, flags, remote, known) = query(file)?;
    if !known {
        return Err(io::Error::other(
            "無法確認輸出位置的檔案系統能力，已拒絕轉檔",
        ));
    }
    classify(&fs, flags, remote, known)
}
/// 控制代碼目前實際對應的路徑。本機磁碟去掉 `\\?\` 字首、UNC 還原成 `\\server\share`，
/// 讓顯示與子程序使用一般路徑；檔案操作仍以控制代碼進行。
pub(super) fn final_path(file: &File) -> io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            file: *mut c_void,
            path: *mut u16,
            size: u32,
            flags: u32,
        ) -> u32;
    }
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    let text = std::ffi::OsString::from_wide(&buffer[..length])
        .to_string_lossy()
        .into_owned();
    Ok(std::path::PathBuf::from(
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            match text.strip_prefix(r"\\?\") {
                Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
                _ => text,
            }
        },
    ))
}
/// 以控制代碼查詢呼叫者可用的剩餘空間（FileFsFullSizeInformation；不是硬配額）。
pub(super) fn free_bytes(file: &File) -> io::Result<u64> {
    let mut status = IoStatus {
        status: 0,
        information: 0,
    };
    // { TotalUnits, CallerAvailableUnits, ActualAvailableUnits: i64, SectorsPerUnit, BytesPerSector: u32 }
    let mut info = [0u64; 4];
    let result = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            &mut status,
            info.as_mut_ptr().cast(),
            32,
            7,
        )
    };
    if result < 0 {
        return Err(io::Error::other(format!(
            "無法查詢磁碟剩餘空間（NTSTATUS {result:#010x}）"
        )));
    }
    let unit = (info[3] & 0xffff_ffff).saturating_mul(info[3] >> 32);
    Ok(info[1].saturating_mul(unit))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_acl_and_non_ntfs_sources_are_copied_and_unknown_local_fails_closed() {
        for fs in ["FAT", "FAT32", "exFAT"] {
            assert_eq!(
                classify(fs, 0, false, true).unwrap(),
                Protection::CopyFirst(fs.into())
            );
        }
        // 名稱是 NTFS 但缺少永久 ACL 旗標：不以名稱推斷，仍先複製。
        assert!(matches!(
            classify("NTFS", 0, false, true).unwrap(),
            Protection::CopyFirst(_)
        ));
        assert!(classify("", 0, false, false).is_err());
    }
    #[test]
    fn only_local_ntfs_with_acl_is_passed_directly() {
        assert_eq!(
            classify("NTFS", 8, false, true).unwrap(),
            Protection::LocalNtfs
        );
        assert!(matches!(
            classify("ReFS", 8, false, true).unwrap(),
            Protection::CopyFirst(_)
        ));
        for (fs, flags, known) in [("NTFS", 8, true), ("", 0, false), ("FAT32", 0, true)] {
            assert!(matches!(
                classify(fs, flags, true, known).unwrap(),
                Protection::CopyFirst(name) if name.starts_with("遠端")
            ));
        }
    }
    #[test]
    fn real_file_and_directory_capabilities_are_observed_by_handle_without_writes() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("source");
        std::fs::write(&p, b"readonly").unwrap();
        let f = File::open(&p).unwrap();
        // 開啟後改名：判定仍屬於已開啟的物件，不依賴路徑。
        std::fs::rename(&p, root.path().join("moved")).unwrap();
        assert_eq!(check(&f).unwrap(), Protection::LocalNtfs);
        let dir = std::fs::OpenOptions::new()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(root.path())
            .unwrap();
        assert_eq!(check_output(&dir).unwrap(), Protection::LocalNtfs);
        drop(f);
        assert_eq!(
            std::fs::read(root.path().join("moved")).unwrap(),
            b"readonly"
        );
    }
}
