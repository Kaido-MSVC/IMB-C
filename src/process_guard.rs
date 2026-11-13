//! Windows Job Object：工作結束或父程序退出時終止已納管的子程序樹。
use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, size: u32)
    -> i32;
    #[cfg(test)]
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
    fn QueryInformationJobObject(
        job: *mut c_void,
        class: i32,
        info: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
    #[cfg(test)]
    fn GetProcessIdOfThread(thread: *mut c_void) -> u32;
    #[cfg(test)]
    fn ResumeThread(thread: *mut c_void) -> u32;
}
#[repr(C)]
#[derive(Default)]
struct Accounting {
    times: [i64; 4],
    page_faults: u32,
    total: u32,
    active: u32,
    terminated: u32,
}
// JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION：基本計數 + IO_COUNTERS。
#[repr(C)]
#[derive(Default)]
struct IoAccounting {
    basic: Accounting,
    read_operations: u64,
    write_operations: u64,
    other_operations: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}
pub(super) struct ProcessGuard(OwnedHandle);
impl ProcessGuard {
    pub(super) fn handle(&self) -> *mut c_void {
        self.0.as_raw_handle()
    }
    /// 整個 Job（含後代）由核心累計的寫入位元組，涵蓋任何路徑、已刪除仍開啟的檔案與管線。
    /// 不含記憶體對應頁面由系統回寫的資料；這是累計 I/O 觀察，不是檔案系統硬配額。
    pub(super) fn write_bytes(&self) -> io::Result<u64> {
        let mut info = IoAccounting::default();
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                8, // JobObjectBasicAndIoAccountingInformation
                (&mut info as *mut IoAccounting).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.write_bytes)
    }
    #[cfg(test)]
    pub(super) fn peak_committed(&self) -> io::Result<usize> {
        let mut info = ExtendedLimits::default();
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                9,
                (&mut info as *mut ExtendedLimits).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.peak_job)
    }
    #[cfg(test)]
    pub(super) fn spawn(
        &self,
        command: &mut std::process::Command,
    ) -> io::Result<std::process::Child> {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        let mut child = command.spawn()?;
        let result = self
            .assign(&child)
            .and_then(|_| Self::resume_primary(&child));
        if let Err(e) = result {
            self.terminate();
            let _ = child.kill();
            let wait = child.wait().map(|_| ());
            let tree = self.stop_and_wait();
            return Err(spawn_failure(e, wait, tree));
        }
        Ok(child)
    }
    #[cfg(test)]
    fn resume_primary(child: &std::process::Child) -> io::Result<()> {
        // Rust 1.100 穩定的 ChildExt::main_thread_handle 直接提供 CreateProcess 回傳的
        // 主執行緒控制代碼。取代 1.99.1 時以全系統 Toolhelp 執行緒快照尋找唯一執行緒、
        // 再以 TID 重新開啟的做法：不需列舉其他程序，也沒有以識別碼重新開啟的間隔。
        use std::os::windows::process::ChildExt;
        let thread = child.main_thread_handle();
        if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != child.id() {
            return Err(io::Error::other("主執行緒程序識別不符"));
        }
        // 只恢復一次：暫停計數必須恰為 1（CREATE_SUSPENDED），否則不放行。
        if unsafe { ResumeThread(thread.as_raw_handle()) } != 1 {
            return Err(io::Error::other("主執行緒恢復狀態不符，已停止工作"));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn new() -> io::Result<Self> {
        Self::with_memory_limit(6usize * 1024 * 1024 * 1024)
    }
    pub(super) fn with_memory_limit(memory: usize) -> io::Result<Self> {
        // 空安全屬性使控制代碼不被子程序繼承，父程序退出會關閉最後持有者。
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let guard = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = ExtendedLimits::default();
        limits.basic.flags = 0x2000 | 0x200 | 0x8; // KILL_ON_JOB_CLOSE | JOB_MEMORY | ACTIVE_PROCESS
        limits.basic.active_processes = 8;
        limits.job_memory = memory;
        // JobObjectExtendedLimitInformation = 9；結構符合 Windows x64 SDK。
        if unsafe {
            SetInformationJobObject(
                handle,
                9,
                (&limits as *const ExtendedLimits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(guard)
    }
    #[cfg(test)]
    pub(super) fn assign(&self, child: &std::process::Child) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub(super) fn terminate(&self) {
        unsafe {
            TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
    pub(super) fn stop_and_wait(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let started = std::time::Instant::now();
        loop {
            let mut info = Accounting::default();
            if unsafe {
                QueryInformationJobObject(
                    self.0.as_raw_handle(),
                    1,
                    (&mut info as *mut Accounting).cast(),
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if info.active == 0 {
                return Ok(());
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                return Err(io::Error::other("等待子程序樹停止逾時"));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
fn spawn_failure(primary: io::Error, wait: io::Result<()>, tree: io::Result<()>) -> io::Error {
    if wait.is_ok() && tree.is_ok() {
        return primary;
    }
    #[derive(Debug)]
    struct Failure {
        primary: io::Error,
        details: String,
    }
    impl std::fmt::Display for Failure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}{}", self.primary, self.details)
        }
    }
    impl std::error::Error for Failure {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.primary)
        }
    }
    let mut details = String::new();
    if let Err(e) = wait {
        details.push_str(&format!("；等待子程序失敗：{e}"));
    }
    if let Err(e) = tree {
        details.push_str(&format!("；無法確認程序樹停止：{e}"));
    }
    io::Error::new(primary.kind(), Failure { primary, details })
}

#[cfg(test)]
mod tests {
    #[test]
    fn spawn_failure_keeps_primary_and_both_cleanup_errors() {
        let error = super::spawn_failure(
            std::io::Error::from_raw_os_error(5),
            Err(std::io::Error::other("wait fixture")),
            Err(std::io::Error::other("tree fixture")),
        );
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let text = error.to_string();
        assert!(
            text.contains("os error 5")
                && text.contains("wait fixture")
                && text.contains("tree fixture")
        );
        let original = error
            .get_ref()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(original.raw_os_error(), Some(5));
        assert_eq!(
            super::spawn_failure(std::io::Error::from_raw_os_error(5), Ok(()), Ok(()))
                .raw_os_error(),
            Some(5)
        );
    }
    use super::*;
    fn shell() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }
    #[test]
    fn terminates_descendant_before_cleanup() {
        use std::io::{BufRead, BufReader};
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let guard = ProcessGuard::new().unwrap();
        let mut child = Command::new(shell()).args(["-NoProfile", "-NonInteractive", "-Command",
            "$p=Start-Process -WindowStyle Hidden -PassThru powershell.exe -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 30'; [Console]::WriteLine($p.Id); [Console]::Out.Flush(); Start-Sleep -Seconds 30"])
            .creation_flags(0x08000000).stdout(Stdio::piped()).spawn().unwrap();
        guard.assign(&child).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.trim().parse::<u32>().is_ok(), "{line}");
        guard.stop_and_wait().unwrap();
        assert!(!child.wait().unwrap().success());
        // stop_and_wait 查詢整棵程序樹的 active == 0，包含後代。
    }
    #[test]
    #[ignore = "Subprocess fixture only; run memory_limit_denies_large_commit"]
    fn memory_child_fixture() {
        let Some(gate) = std::env::var_os("IMBC_SECURITY_MEMORY_CHILD") else {
            return;
        };
        let started = std::time::Instant::now();
        while !std::path::Path::new(&gate).is_file() {
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut data = Vec::<u8>::new();
        assert!(data.try_reserve_exact(512 * 1024 * 1024).is_err());
    }
    #[test]
    fn memory_limit_denies_large_commit() {
        let dir = tempfile::tempdir().unwrap();
        let gate = dir.path().join("ready");
        let guard = ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["memory_child_fixture", "--ignored", "--test-threads=1"])
            .env("IMBC_SECURITY_MEMORY_CHILD", &gate)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        guard.assign(&child).unwrap();
        std::fs::write(gate, b"ready").unwrap();
        assert!(child.wait().unwrap().success());
        guard.stop_and_wait().unwrap();
    }
    #[test]
    #[ignore = "Subprocess fixture only; run parent_exit_closes_job_and_terminates_child"]
    fn parent_exit_fixture() {
        use std::io::{BufRead, BufReader};
        let Some(root) = std::env::var_os("IMBC_SECURITY_EXIT_CHILD") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let guard = ProcessGuard::new().unwrap();
        let mut child = std::process::Command::new(shell())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::WriteLine($PID); [Console]::Out.Flush(); Start-Sleep -Seconds 30",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        guard.assign(&child).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        std::fs::write(root.join("pid"), line).unwrap();
        let started = std::time::Instant::now();
        while !root.join("exit").is_file() {
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::process::exit(0); // 刻意略過解構器，檢查 OS 關閉控制代碼。
    }
    #[test]
    fn parent_exit_closes_job_and_terminates_child() {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut parent = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["parent_exit_fixture", "--ignored", "--test-threads=1"])
            .env("IMBC_SECURITY_EXIT_CHILD", dir.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while !dir.path().join("pid").exists() {
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let pid = std::fs::read_to_string(dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let handle = unsafe { OpenProcess(0x00100000, 0, pid) }; // SYNCHRONIZE
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        std::fs::write(dir.path().join("exit"), b"exit").unwrap();
        assert!(parent.wait().unwrap().success());
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            0
        );
    }
    #[test]
    fn sdk_x64_layout() {
        assert_eq!(std::mem::size_of::<BasicLimits>(), 64);
        assert_eq!(std::mem::size_of::<ExtendedLimits>(), 144);
        assert_eq!(std::mem::size_of::<Accounting>(), 48);
        assert_eq!(std::mem::size_of::<IoAccounting>(), 96);
    }
    // 1.100 main_thread_handle：暫停建立 → 納管 → 只恢復一次；子程序在 Job 內實際執行。
    #[test]
    fn suspended_spawn_resumes_main_thread_inside_job() {
        let guard = ProcessGuard::new().unwrap();
        let mut command = std::process::Command::new(shell());
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", "exit 7"])
            .stdout(std::process::Stdio::null());
        let mut child = guard.spawn(&mut command).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));
        guard.stop_and_wait().unwrap();
    }
    // 核心 I/O 計數涵蓋管線與任意路徑的檔案寫入；用於扣除管線後估算原生寫入。
    #[test]
    fn job_write_bytes_count_pipe_and_file_writes() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("outside.bin");
        let guard = ProcessGuard::new().unwrap();
        let script = format!(
            "$o=[Console]::OpenStandardOutput(); $b=New-Object byte[] 1048576; for($i=0;$i -lt 4;$i++){{ $o.Write($b,0,$b.Length) }}; $o.Flush(); [IO.File]::WriteAllBytes('{}', (New-Object byte[] 8388608))",
            file.display()
        );
        let mut command = std::process::Command::new(shell());
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdout(std::process::Stdio::piped());
        let mut child = guard.spawn(&mut command).unwrap();
        let mut piped = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut piped)
            .unwrap();
        assert!(child.wait().unwrap().success());
        let written = guard.write_bytes().unwrap();
        guard.stop_and_wait().unwrap();
        assert_eq!(piped.len(), 4 * 1048576);
        assert_eq!(std::fs::metadata(&file).unwrap().len(), 8388608);
        // 4 MiB 管線 + 8 MiB 檔案 都必須被計入；PowerShell 啟動本身另有少量寫入。
        let native = written.saturating_sub(piped.len() as u64);
        assert!(native >= 8388608, "written={written} piped={}", piped.len());
        assert!(written >= 12 * 1048576, "written={written}");
    }
}
