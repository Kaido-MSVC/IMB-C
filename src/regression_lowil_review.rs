//! Claude Low IL 審查的聚焦回歸；所有寫入僅限本測試新建的目錄。
use super::*;
use std::{fs, io::Write};
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
}
fn input(text: &str) -> File {
    let mut file = tempfile::tempfile_in(controlled_temp::preferred_root().unwrap()).unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file
}
fn command(scratch: &Path) -> Command {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(&exe);
    child_environment(&mut cmd, exe.parent().unwrap(), scratch, scratch).unwrap();
    cmd.current_dir(exe.parent().unwrap()).args([
        "--exact",
        "engine::regression_lowil_review::review_child_fixture",
        "--ignored",
        "--nocapture",
    ]);
    cmd
}
fn finish(
    child: &mut low_integrity::Child,
    guard: &process_guard::ProcessGuard,
) -> (std::process::ExitStatus, String) {
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    if child.try_wait().unwrap().is_none() {
        guard.terminate();
    }
    let status = child.wait().unwrap();
    guard.stop_and_wait().unwrap();
    let stopped = AtomicBool::new(true);
    let mut text = String::new();
    let mut errors = String::new();
    engine_limits::PipeReader::new(child.stdout.take().unwrap(), &stopped)
        .read_to_string(&mut text)
        .unwrap();
    engine_limits::PipeReader::new(child.stderr.take().unwrap(), &stopped)
        .read_to_string(&mut errors)
        .unwrap();
    assert!(status.success(), "{status}\n{text}\n{errors}");
    (status, text)
}
fn low_dir(sandbox: &low_integrity::Sandbox, prefix: &str) -> controlled_temp::ControlledDir {
    let dir =
        controlled_temp::ControlledDir::create(&controlled_temp::preferred_root().unwrap(), prefix)
            .unwrap();
    dir.allow_sandbox(sandbox).unwrap();
    dir
}

// 僅建立測試自己的 mount-point reparse point，不經命令殼層或修改全域設定。
fn junction(link: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            file: *mut std::ffi::c_void,
            code: u32,
            input: *const u8,
            input_size: u32,
            output: *mut u8,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let target = display_path(target);
    let substitute: Vec<u16> = format!(r"\??\{target}").encode_utf16().collect();
    let print: Vec<u16> = target.encode_utf16().collect();
    let length = 8 + (substitute.len() + 1 + print.len() + 1) * 2;
    assert!(length < 16384);
    let mut data = Vec::new();
    data.extend(0xa0000003u32.to_le_bytes());
    for v in [
        length as u16,
        0,
        0,
        (substitute.len() * 2) as u16,
        ((substitute.len() + 1) * 2) as u16,
        (print.len() * 2) as u16,
    ] {
        data.extend(v.to_le_bytes());
    }
    for v in substitute.into_iter().chain([0]).chain(print).chain([0]) {
        data.extend(v.to_le_bytes());
    }
    fs::create_dir(link)?;
    let file = File::options()
        .access_mode(0x40000000)
        .share_mode(7)
        .custom_flags(0x02200000)
        .open(link)?;
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x900a4,
            data.as_ptr(),
            data.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[test]
#[ignore = "Subprocess fixture only"]
fn review_child_fixture() {
    assert_eq!(
        low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap(),
        4096
    );
    let admin = low_integrity::admin_attributes(unsafe { GetCurrentProcess() }).unwrap();
    assert!(admin.is_none_or(|a| a & 6 == 0 && a & 0x10 != 0));
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let mut lines = text.lines();
    match lines.next().unwrap() {
        "admin" => println!("CHILD admin={admin:?} IL=4096"),
        "links" => {
            let target = PathBuf::from(lines.next().unwrap());
            let own = PathBuf::from(std::env::var_os("TEMP").unwrap()).join("nested");
            fs::create_dir(&own).unwrap();
            let link = own.join("junction");
            junction(&link, &target).unwrap();
            fs::hard_link(target.join("sentinel"), own.join("hardlink")).unwrap();
            assert_eq!(fs::read(link.join("sentinel")).unwrap(), b"preserve-target");
            println!("LOW_LINKS junction=true hardlink=true");
        }
        "cross" => {
            let target = PathBuf::from(lines.next().unwrap());
            let pid: u32 = lines.next().unwrap().parse().unwrap();
            fs::write(target, b"cross-job-tamper").unwrap();
            let handle = unsafe { OpenProcess(0x69, 0, pid) };
            println!(
                "CROSS_JOB write=true process_modify_handle={}",
                !handle.is_null()
            );
            if !handle.is_null() {
                unsafe { CloseHandle(handle) };
            }
        }
        "outside-write" => {
            let target = PathBuf::from(lines.next().unwrap());
            let mut file = File::create(target).unwrap();
            let data = vec![7; 1048576];
            for _ in 0..256 {
                file.write_all(&data).unwrap();
                file.flush().unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        "crash" => {
            unsafe { TerminateProcess(GetCurrentProcess(), 37) };
            panic!("TerminateProcess 不應返回");
        }
        "sleep" => std::thread::sleep(Duration::from_secs(15)),
        _ => panic!("未知測試模式"),
    }
}

#[test]
fn review_m1_admin_disabled_and_parent_unchanged() {
    let before = low_integrity::admin_attributes(unsafe { GetCurrentProcess() }).unwrap();
    let il = low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap();
    let sandbox = low_integrity::Sandbox::new().unwrap();
    let dir = low_dir(&sandbox, "imb-c-admin-");
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let mut child = sandbox
        .spawn(&command(dir.path()), &input("admin\n"), guard.handle())
        .unwrap();
    let (_, text) = finish(&mut child, &guard);
    assert_eq!(
        low_integrity::admin_attributes(unsafe { GetCurrentProcess() }).unwrap(),
        before
    );
    assert_eq!(
        low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap(),
        il
    );
    eprintln!("PARENT admin={before:?} IL={il}; {text}");
    dir.close().unwrap();
}

// T-2 根因回歸（2026-11-13）：已提升權限系統管理員 Token 的預設擁有者／預設 DACL
// 只授權 Administrators 與 SYSTEM。Administrators 改為 deny-only 後，Low IL 子程序無法
// 存取自身以預設 DACL 建立的物件，DLL 初始化失敗（0xC0000142）。以一般權限就能模擬此 Token。
#[test]
fn review_t2_elevated_style_default_dacl_child_still_starts() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    // 子程序繼承錯誤模式：失敗時直接結束，不在桌面留下載入器錯誤對話方塊。
    let previous = unsafe { SetErrorMode(0x8003) };
    let sandbox = low_integrity::Sandbox::from_elevated_style_token().unwrap();
    let dir = low_dir(&sandbox, "imb-c-t2dacl-");
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let spawned = sandbox.spawn(&command(dir.path()), &input("admin\n"), guard.handle());
    unsafe { SetErrorMode(previous) };
    let mut child = spawned.unwrap();
    let (_, text) = finish(&mut child, &guard);
    assert!(
        text.contains("CHILD admin=") && text.contains("IL=4096"),
        "{text}"
    );
    eprintln!("T2-DACL {text}");
    dir.close().unwrap();
}

#[test]
#[ignore = "T-2 requires an already elevated terminal; do not auto-elevate"]
fn review_t2_elevated_administrators_sid() {
    assert!(
        low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap() >= 12288,
        "必須從已提升權限的終端機執行，未驗證不算通過"
    );
    assert!(
        low_integrity::admin_attributes(unsafe { GetCurrentProcess() })
            .unwrap()
            .is_some_and(|a| a & 4 != 0)
    );
    review_m1_admin_disabled_and_parent_unchanged();
}

#[test]
fn review_t1_low_created_links_cleanup_keeps_target() {
    let sandbox = low_integrity::Sandbox::new().unwrap();
    // 已建立並降低標籤，再釋放目錄鎖；模擬工作外的可寫測試位置。
    let target = low_dir(&sandbox, "imb-c-link-target-").keep();
    let own = low_dir(&sandbox, "imb-c-link-own-");
    fs::write(target.join("sentinel"), b"preserve-target").unwrap();
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let mut child = sandbox
        .spawn(
            &command(own.path()),
            &input(&format!("links\n{}\n", target.display())),
            guard.handle(),
        )
        .unwrap();
    let (_, text) = finish(&mut child, &guard);
    assert!(text.contains("junction=true hardlink=true"));
    use std::os::windows::fs::MetadataExt;
    assert_eq!(
        fs::symlink_metadata(own.path().join("nested/junction"))
            .unwrap()
            .file_attributes()
            & 0x410,
        0x410
    );
    own.close().unwrap();
    assert_eq!(
        fs::read(target.join("sentinel")).unwrap(),
        b"preserve-target"
    );
    eprintln!("T1 {text} target preserved after cleanup");
    fs::remove_file(target.join("sentinel")).unwrap();
    fs::remove_dir(&target).unwrap();
}

#[test]
fn review_m2_cross_job_file_and_process_access_observation() {
    let sandbox = low_integrity::Sandbox::new().unwrap();
    let target = low_dir(&sandbox, "imb-c-peer-");
    let own = low_dir(&sandbox, "imb-c-observer-");
    let file = target.path().join("dummy-output");
    fs::write(&file, b"original").unwrap();
    let peer_guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let mut peer = sandbox
        .spawn(
            &command(target.path()),
            &input("sleep\n"),
            peer_guard.handle(),
        )
        .unwrap();
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let mut child = sandbox
        .spawn(
            &command(own.path()),
            &input(&format!("cross\n{}\n{}\n", file.display(), peer.id())),
            guard.handle(),
        )
        .unwrap();
    let (_, text) = finish(&mut child, &guard);
    peer_guard.terminate();
    peer.wait().unwrap();
    peer_guard.stop_and_wait().unwrap();
    assert_eq!(fs::read(file).unwrap(), b"cross-job-tamper");
    eprintln!("M2 {text}");
    own.close().unwrap();
    target.close().unwrap();
}

#[test]
fn review_m2_pipe_writer_held_outside_job_does_not_hang() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let source = job_file().unwrap();
    let output = job_file().unwrap();
    low_integrity::TEST_HOLD_PIPE.with(|v| v.set(true));
    let started = Instant::now();
    let result = run(
        &windows_directory().unwrap().join("System32/cmd.exe"),
        &["/d", "/c", "exit 0"],
        &source,
        &output,
        &AtomicBool::new(false),
    );
    low_integrity::TEST_HOLD_PIPE.with(|v| v.set(false));
    low_integrity::TEST_HELD_PIPE.with(|v| v.borrow_mut().take());
    assert!(started.elapsed() < Duration::from_secs(6));
    let error = result.unwrap_err();
    assert!(error.contains("管線仍未關閉"), "{error}");
    drop((source, output));
    assert!(job.finish::<()>(Err(error)).is_err());
    assert!(!root.exists());
}

#[test]
fn review_l2_confirmed_failed_launch_cleans_low_scratch() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let source = job_file().unwrap();
    let output = job_file().unwrap();
    low_integrity::TEST_REJECT_BEFORE_RESUME.with(|v| v.set(true));
    let result = run(
        &windows_directory().unwrap().join("System32/cmd.exe"),
        &["/d", "/c", "exit 0"],
        &source,
        &output,
        &AtomicBool::new(false),
    );
    low_integrity::TEST_REJECT_BEFORE_RESUME.with(|v| v.set(false));
    let error = result.unwrap_err();
    assert!(
        error.contains("Low IL 驗證失敗") && !error.contains("暫存保留"),
        "{error}"
    );
    drop((source, output));
    assert!(job.finish::<()>(Err(error)).is_err());
    assert!(!root.exists());
}

#[test]
fn review_t3_low_writable_outside_invocation_hits_write_monitor() {
    let sandbox = low_integrity::Sandbox::new().unwrap();
    let outside = low_dir(&sandbox, "imb-c-outside-");
    let target = outside.path().join("bounded.bin");
    let job = Job::new().unwrap();
    let root = job_dir();
    let source = input(&format!("outside-write\n{}\n", target.display()));
    let output = job_file().unwrap();
    let exe = std::env::current_exe().unwrap();
    engine_limits::TEST_NATIVE_WRITE_LIMIT.with(|v| v.set(4 * 1048576));
    let result = run(
        &exe,
        &[
            "--exact",
            "engine::regression_lowil_review::review_child_fixture",
            "--ignored",
            "--nocapture",
        ],
        &source,
        &output,
        &AtomicBool::new(false),
    );
    engine_limits::TEST_NATIVE_WRITE_LIMIT.with(|v| v.set(engine_limits::NATIVE_WRITE_LIMIT));
    let error = result.unwrap_err();
    assert!(error.contains("累計寫入"), "{error}");
    let size = fs::metadata(&target).unwrap().len();
    assert!(size > 4 * 1048576 && size < 64 * 1048576, "{size}");
    eprintln!("T3 outside native bytes={size}; monitor stopped process");
    drop((source, output));
    assert!(job.finish::<()>(Err(error)).is_err());
    assert!(!root.exists());
    outside.close().unwrap();
}

#[test]
fn review_low_child_abrupt_exit_cleans_scratch() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let source = input("crash\n");
    let output = job_file().unwrap();
    let result = run(
        &std::env::current_exe().unwrap(),
        &[
            "--exact",
            "engine::regression_lowil_review::review_child_fixture",
            "--ignored",
            "--nocapture",
        ],
        &source,
        &output,
        &AtomicBool::new(false),
    );
    let error = result.unwrap_err();
    assert!(error.contains("37"), "{error}");
    drop((source, output));
    assert!(job.finish::<()>(Err(error)).is_err());
    assert!(!root.exists());
}
