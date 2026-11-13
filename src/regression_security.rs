use super::*;
use std::{fs, io::Write};

#[test]
fn security_lowil_rejected_launch_stops_suspended_process() {
    let sandbox = low_integrity::Sandbox::new().unwrap();
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let input = tempfile::tempfile_in(controlled_temp::preferred_root().unwrap()).unwrap();
    let mut command = Command::new(windows_directory().unwrap().join("System32/cmd.exe"));
    command.env_clear().args(["/d", "/c", "exit 0"]);
    low_integrity::TEST_REJECT_BEFORE_RESUME.with(|v| v.set(true));
    let result = sandbox.spawn(&command, &input, guard.handle());
    low_integrity::TEST_REJECT_BEFORE_RESUME.with(|v| v.set(false));
    let error = match result {
        Ok(_) => panic!("拒絕的程序不應恢復"),
        Err(e) => e,
    };
    assert!(error.to_string().contains("Low IL 驗證失敗"));
    guard.stop_and_wait().unwrap();
}

#[test]
#[ignore = "Subprocess fixture only; run security_lowil_blocks_medium_writes"]
fn lowil_child_fixture() {
    assert_eq!(
        low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap(),
        4096
    );
    let font = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("Fonts/segoeui.ttf");
    assert!(!fs::read(font).unwrap().is_empty());
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let mut paths = text.lines();
    let victim = PathBuf::from(paths.next().unwrap());
    assert_eq!(fs::read(&victim).unwrap(), b"medium-sentinel");
    assert_eq!(
        fs::write(&victim, b"changed").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        fs::remove_file(&victim).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(fs::rename(&victim, victim.with_extension("renamed")).is_err());
    assert_eq!(
        fs::write(victim.with_extension("new"), b"new")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let own = PathBuf::from(std::env::var_os("TEMP").unwrap()).join("low-child.bin");
    fs::write(&own, b"low-ok").unwrap();
    assert_eq!(fs::read(&own).unwrap(), b"low-ok");
    if let Some(unc) = paths.next() {
        // 此斷言僅適用本機 SMB 實測，不能推論所有遠端伺服器。
        assert!(
            fs::write(unc, b"changed-via-smb").is_err(),
            "Low IL 不應能修改本機 SMB 的一般檔案"
        );
        println!("lowil-local-smb-write-denied");
    }
    // 繼承 stdin 的現有控制代碼也必須沒有寫入權限。
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn WriteFile(
            file: *mut std::ffi::c_void,
            data: *const u8,
            size: u32,
            written: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let mut written = 0;
    assert_eq!(
        unsafe {
            WriteFile(
                GetStdHandle(-10i32 as u32),
                b"X".as_ptr(),
                1,
                &mut written,
                std::ptr::null_mut(),
            )
        },
        0
    );
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(5));
    assert!(
        Vec::<u8>::new()
            .try_reserve_exact(512 * 1024 * 1024)
            .is_err()
    );
    // 8 個作用中程序包含自己；第 8 個後代須由 Job 拒絕。
    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for _ in 0..7 {
        children.push(
            Command::new(&exe)
                .args([
                    "--exact",
                    "engine::regression_security::lowil_sleep_fixture",
                    "--ignored",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    assert!(
        Command::new(&exe)
            .args([
                "--exact",
                "engine::regression_security::lowil_sleep_fixture",
                "--ignored"
            ])
            .spawn()
            .is_err()
    );
    for mut child in children {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    println!("lowil-boundary-ok");
}

#[test]
#[ignore = "Subprocess fixture only"]
fn lowil_sleep_fixture() {
    std::thread::sleep(Duration::from_secs(15));
}

#[test]
fn security_lowil_blocks_medium_writes() {
    let before = low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap();
    assert!(before >= 8192);
    let root = controlled_temp::preferred_root().unwrap();
    let victim_dir = tempfile::tempdir_in(&root).unwrap();
    let victim = victim_dir.path().join("victim.txt");
    fs::write(&victim, b"medium-sentinel").unwrap();
    let unc_dir = std::env::var_os("IMBC_TEST_UNC_ROOT").map(|p| tempfile::tempdir_in(p).unwrap());
    let unc_victim = unc_dir.as_ref().map(|d| d.path().join("medium-unc.txt"));
    if let Some(p) = &unc_victim {
        fs::write(p, b"medium-smb-sentinel").unwrap();
    }
    let sandbox = low_integrity::Sandbox::new().unwrap();
    let scratch = controlled_temp::ControlledDir::create(&root, "imb-c-lowil-test-").unwrap();
    scratch.allow_sandbox(&sandbox).unwrap();
    let mut source = tempfile::tempfile_in(victim_dir.path()).unwrap();
    source
        .write_all(
            format!(
                "{}\n{}",
                victim.display(),
                unc_victim
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            )
            .as_bytes(),
        )
        .unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut command = Command::new(&exe);
    child_environment(
        &mut command,
        exe.parent().unwrap(),
        scratch.path(),
        scratch.path(),
    )
    .unwrap();
    command.current_dir(exe.parent().unwrap()).args([
        "--exact",
        "engine::regression_security::lowil_child_fixture",
        "--ignored",
        "--nocapture",
    ]);
    let guard = process_guard::ProcessGuard::with_memory_limit(256 * 1024 * 1024).unwrap();
    let mut child = sandbox.spawn(&command, &source, guard.handle()).unwrap();
    let mut output = String::new();
    let mut errors = String::new();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        guard.terminate();
    }
    let status = child.wait().unwrap();
    guard.stop_and_wait().unwrap();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    assert!(status.success(), "{status}: {output}\n{errors}");
    assert!(output.contains("lowil-boundary-ok"));
    assert_eq!(fs::read(&victim).unwrap(), b"medium-sentinel");
    if let Some(p) = &unc_victim {
        assert_eq!(fs::read(p).unwrap(), b"medium-smb-sentinel");
    }
    eprintln!(
        "LOWIL boundary: medium writes denied; private temp writable; stdin readonly; parent IL={before}; Job memory/process limits enforced; local SMB tested={}",
        unc_victim.is_some()
    );
    assert_eq!(
        low_integrity::integrity(unsafe { GetCurrentProcess() }).unwrap(),
        before
    );
    assert_eq!(
        fs::read(scratch.path().join("low-child.bin")).unwrap(),
        b"low-ok"
    );
    scratch.close().unwrap();
}

#[test]
fn security_live_child_reader_panics_and_unknown_stop_retain_all_scratch() {
    for which in [1, 2, 3] {
        let job = Job::new().unwrap();
        let root = job_dir();
        fs::write(root.join("sentinel"), b"preserve").unwrap();
        let input = job_file().unwrap();
        let output = job_file().unwrap();
        TEST_READER_PANIC.with(|v| v.set(which));
        TEST_TREE_FAILURE.with(|v| v.set(true));
        TEST_TREE_CONFIRMED.with(|v| v.set(false));
        let shell = windows_directory()
            .unwrap()
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let started = Instant::now();
        let result = run(
            &shell,
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.WriteLine('ready'); [Console]::Error.WriteLine('diagnostic'); Start-Sleep -Seconds 30",
            ],
            &input,
            &output,
            &AtomicBool::new(false),
        );
        TEST_READER_PANIC.with(|v| v.set(0));
        TEST_TREE_FAILURE.with(|v| v.set(false));
        assert!(TEST_TREE_CONFIRMED.with(|v| v.get()));
        assert!(started.elapsed() < Duration::from_secs(10));
        let error = result.unwrap_err();
        assert!(
            error.contains("injected tree confirmation failure") && error.contains("保留"),
            "{error}"
        );
        if which & 1 != 0 {
            assert!(error.contains("stdout 讀取執行緒異常"));
        }
        if which & 2 != 0 {
            assert!(error.contains("stderr 讀取執行緒異常"));
        }
        drop((input, output));
        let error = job.finish::<()>(Err(error)).unwrap_err();
        assert!(error.contains("保留"));
        assert_eq!(fs::read(root.join("sentinel")).unwrap(), b"preserve");
        // 注入失敗前的真實 Job 查詢已確認 active=0，故僅清除此案例的新目錄。
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn security_reader_panics_never_bypass_unknown_tree_retention() {
    for which in [1, 2, 3] {
        let job = Job::new().unwrap();
        let root = job_dir();
        let scratch =
            JOB_STORAGE.with(|v| v.borrow().as_ref().unwrap().child("imb-c-join-").unwrap());
        fs::write(scratch.path().join("sentinel"), b"keep until confirmed").unwrap();
        let mut invocation = Invocation {
            scratch: Some(scratch),
            stopped: true,
        };
        let joined = std::cell::Cell::new(0);
        let error = std::thread::scope(|scope| {
            let out = scope.spawn(move || {
                if which & 1 != 0 {
                    panic!("stdout fixture");
                }
                Ok(())
            });
            let stderr = scope.spawn(move || {
                if which & 2 != 0 {
                    panic!("stderr fixture");
                }
                Ok(())
            });
            finish_child_io(
                &mut invocation.stopped,
                Err("original conversion error".into()),
                Err(std::io::Error::other("tree query fixture")),
                || {
                    joined.set(joined.get() + 1);
                    out.join()
                },
                || {
                    joined.set(joined.get() + 1);
                    stderr.join()
                },
            )
        })
        .unwrap_err();
        assert_eq!(joined.get(), 2);
        assert!(!invocation.stopped);
        assert!(
            error.contains("original conversion error") && error.contains("tree query fixture")
        );
        if which & 1 != 0 {
            assert!(error.contains("stdout 讀取執行緒異常"));
        }
        if which & 2 != 0 {
            assert!(error.contains("stderr 讀取執行緒異常"));
        }
        drop(invocation);
        let error = job.finish::<()>(Err(error)).unwrap_err();
        assert!(error.contains("保留") && root.exists());
        // 本案例沒有啟動 OS 子程序，兩個讀取執行緒均已 join；只清除此案例的新目錄。
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn security_shared_directory_retention_survives_last_arc_and_keeps_primary_error() {
    for fail in [false, true] {
        let job = Job::new().unwrap();
        let root = job_dir();
        let hold = JOB_STORAGE.with(|v| v.borrow().as_ref().unwrap().clone());
        fs::write(root.join("sentinel"), b"owned").unwrap();
        let result = job.finish_with_warnings(if fail {
            Err("primary conversion failure".into())
        } else {
            Ok(42)
        });
        if fail {
            let error = result.unwrap_err();
            assert!(
                error.contains("primary conversion failure")
                    && error.contains("仍被使用")
                    && error.contains("保留")
            );
        } else {
            let (value, warnings) = result.unwrap();
            assert_eq!(value, 42);
            assert!(
                warnings
                    .iter()
                    .any(|w| w.contains("仍被使用") && w.contains("保留"))
            );
        }
        drop(hold);
        assert_eq!(fs::read(root.join("sentinel")).unwrap(), b"owned");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn security_job_unwind_retains_shared_directory_after_catch() {
    let mut root = PathBuf::new();
    let mut held = None;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _job = Job::new().unwrap();
        root = job_dir();
        held = JOB_STORAGE.with(|v| v.borrow().clone());
        fs::write(root.join("sentinel"), b"owned").unwrap();
        panic!("job fixture unwind");
    }));
    assert!(panic.is_err());
    drop(held);
    assert_eq!(fs::read(root.join("sentinel")).unwrap(), b"owned");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_pump_panic_notifies_controller() {
    struct PanicRead;
    impl std::io::Read for PanicRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("reader fixture");
        }
    }
    struct PanicWrite;
    impl std::io::Write for PanicWrite {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            panic!("writer fixture");
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for write in [false, true] {
        let failed = AtomicBool::new(false);
        let budget = engine_limits::Budget::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if write {
                let _ = engine_limits::pump(
                    &b"a"[..],
                    PanicWrite,
                    16,
                    &budget,
                    &failed,
                    &std::sync::atomic::AtomicU64::new(0),
                );
            } else {
                let _ = engine_limits::pump(
                    PanicRead,
                    Vec::new(),
                    16,
                    &budget,
                    &failed,
                    &std::sync::atomic::AtomicU64::new(0),
                );
            }
        }));
        assert!(result.is_err());
        assert!(failed.load(Ordering::Relaxed));
    }
}

#[test]
fn security_child_environment_is_explicit_and_uses_owned_temp() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let shell = windows_directory()
        .unwrap()
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = Command::new(shell);
    command
        .env("UNRELATED_SECRET", "dummy fixture")
        .env("MAGICK_MAP_LIMIT", "unlimited");
    child_environment(&mut command, Path::new(r"D:\private-engine"), &root, &root).unwrap();
    let explicit: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert!(!explicit.contains_key(std::ffi::OsStr::new("UNRELATED_SECRET")));
    assert!(!explicit.contains_key(std::ffi::OsStr::new("MAGICK_MAP_LIMIT")));
    use std::os::windows::process::CommandExt;
    let output = command.creation_flags(0x08000000).args(["-NoProfile", "-NonInteractive", "-Command",
        "$ErrorActionPreference='Stop'; if ($env:UNRELATED_SECRET -or $env:MAGICK_MAP_LIMIT) { throw 'unexpected environment' }; foreach($n in @('TEMP','TMP','TMPDIR','HOME','USERPROFILE','APPDATA','LOCALAPPDATA')) { if([Environment]::GetEnvironmentVariable($n) -ne $env:MAGICK_TEMPORARY_PATH) { throw $n } }; [Console]::Write('whitelist-ok')"])
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"whitelist-ok");
    job.finish(Ok(())).unwrap();
}

#[test]
fn security_unwind_retains_scratch_when_stop_is_unconfirmed() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let result = std::panic::catch_unwind(|| {
        let scratch =
            JOB_STORAGE.with(|v| v.borrow().as_ref().unwrap().child("imb-c-job-").unwrap());
        let _invocation = Invocation {
            scratch: Some(scratch),
            stopped: false,
        };
        panic!("injected before stop confirmation; no child was started");
    });
    assert!(result.is_err());
    assert!(
        job.finish::<()>(Err("fixture unwind".into()))
            .unwrap_err()
            .contains("保留")
    );
    assert!(root.exists());
    assert!(root.starts_with(controlled_temp::preferred_root().unwrap()));
    // 此測試沒有啟動子程序；僅清除此測試剛建立的已知目錄。
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_directory_replacement_and_junction_are_rejected() {
    let base = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    let sentinel = base.path().join("sentinel");
    fs::write(&sentinel, b"untouched").unwrap();
    let owned = controlled_temp::ControlledDir::create(base.path(), "imb-c-pin-").unwrap();
    let path = owned.path().to_owned();
    assert!(fs::rename(&path, base.path().join("replaced")).is_err());
    assert!(controlled_temp::ControlledDir::create(&base.path().join(".."), "bad-").is_err());
    let link = path.join("external-link");
    std::os::windows::fs::symlink_dir(base.path(), &link).unwrap();
    assert!(controlled_temp::ControlledDir::create(&link, "bad-").is_err());
    owned.close().unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read(sentinel).unwrap(), b"untouched");
}

#[test]
fn security_allocation_diagnostics_keep_native_details() {
    for detail in [
        "std::bad_alloc",
        "memory allocation failed",
        "exit code: 0xe06d7363",
    ] {
        let error = image_stage_error("JXL 編碼", detail.into());
        assert!(
            error.contains("committed-memory") && error.contains(detail),
            "{error}"
        );
    }
}

#[test]
#[ignore = "Requires real ImageMagick runtime; forces disk-backed pixel cache"]
fn security_real_native_cache_uses_owned_preferred_directory() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let job = Job::new().unwrap();
    let root = job_dir();
    assert!(root.starts_with(controlled_temp::preferred_root().unwrap()));
    let mut source = job_file().unwrap();
    source
        .write_all(include_bytes!("../tests/fixtures/seed.png"))
        .unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let output = job_file().unwrap();
    TEST_NATIVE_PEAK.with(|v| v.set(0));
    run(
        &engine,
        &[
            "-limit",
            "memory",
            "1KiB",
            "-limit",
            "map",
            "0MiB",
            "-limit",
            "disk",
            "64MiB",
            "png:-",
            "-resize",
            "1000x1000!",
            "miff:-",
        ],
        &source,
        &output,
        &AtomicBool::new(false),
    )
    .unwrap();
    let peak = TEST_NATIVE_PEAK.with(|v| v.get());
    eprintln!(
        "NATIVE actual_files_peak={peak} owned_root={}",
        root.display()
    );
    assert!(
        peak >= 1000 * 1000 * 16,
        "ImageMagick 沒有在受控目錄產生預期的原生快取：{peak}"
    );
    drop((source, output));
    job.finish(Ok(())).unwrap();
    assert!(!root.exists());
}

#[test]
#[ignore = "Real runtime; verifies pixel-cache disk limit, not a whole-job disk quota"]
fn security_pixel_cache_disk_limit_rejects_and_cleans() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let job = Job::new().unwrap();
    let root = job_dir();
    let mut source = job_file().unwrap();
    source
        .write_all(include_bytes!("../tests/fixtures/seed.png"))
        .unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let output = job_file().unwrap();
    let error = run(
        &engine,
        &[
            "-limit",
            "memory",
            "1KiB",
            "-limit",
            "map",
            "0MiB",
            "-limit",
            "disk",
            "1MiB",
            "png:-",
            "-resize",
            "1000x1000!",
            "miff:-",
        ],
        &source,
        &output,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("cache resources exhausted"), "{error}");
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, include_bytes!("../tests/fixtures/seed.png"));
    drop((source, output));
    job.finish(Ok(())).unwrap();
    assert!(!root.exists());
}

#[test]
#[ignore = "Real AVIF runtime; 16-bit input encodes as 12-bit AVIF, not native 16-bit AV1"]
fn security_avif_8bit_and_16bit_source_to_12bit_icc_roundtrip() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let dir = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    for (name, bytes, depth) in [
        (
            "icc8.png",
            include_bytes!("../tests/fixtures/acceptance-v13/icc8.png").as_slice(),
            8,
        ),
        (
            "icc16.png",
            include_bytes!("../tests/fixtures/v13-icc16.png").as_slice(),
            16,
        ),
    ] {
        let input = dir.path().join(name);
        fs::write(&input, bytes).unwrap();
        let before = sha256(&mut File::open(&input).unwrap()).unwrap();
        let (encoded, encode) = convert_with_options(
            &engine,
            &input,
            dir.path(),
            Format::Avif,
            &Options {
                avif_quality: 100,
                ..Default::default()
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(encode.input_depth, depth);
        let encoded_depth = if depth == 16 { 12 } else { 8 };
        assert_eq!(encode.output_depth, encoded_depth);
        assert!(
            encode.icc && encode.color_verified && encode.warnings.is_empty(),
            "{encode:?}"
        );
        let (_, decode) = convert_with_options(
            &engine,
            &encoded,
            dir.path(),
            Format::Png,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(decode.input_depth, encoded_depth);
        assert_eq!(decode.output_depth, depth);
        assert!(
            decode.exact && decode.icc && decode.color_verified && decode.warnings.is_empty(),
            "{decode:?}"
        );
        assert_eq!(sha256(&mut File::open(&input).unwrap()).unwrap(), before);
        eprintln!(
            "AVIF_REPRESENTATION depth={depth} source_sha256={before} encode={encode:?} decode={decode:?}"
        );
    }
}

#[test]
#[ignore = "Requires isolated ImageMagick core; hostile config remains in test directory"]
fn security_core_ignores_stealth_policy_and_delegate_search_paths() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let foreign = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    fs::write(foreign.path().join("policy.xml"), b"<policymap><policy domain='coder' rights='read|write' pattern='*' stealth='true'/></policymap>").unwrap();
    fs::write(foreign.path().join("delegates.xml"), b"<delegatemap><delegate decode='png' encode='rgb' command='cmd /c exit 99'/></delegatemap>").unwrap();
    let mut command = super::tests::inspect(&engine);
    command
        .current_dir(foreign.path())
        .env("MAGICK_CONFIGURE_PATH", foreign.path())
        .env("MAGICK_HOME", foreign.path())
        .env("HOME", foreign.path())
        .env("MAGICK_CODER_MODULE_PATH", foreign.path())
        .env("MAGICK_FILTER_MODULE_PATH", foreign.path());
    let listing = command.args(["-list", "policy"]).output().unwrap();
    assert!(listing.status.success());
    let listing = String::from_utf8_lossy(&listing.stdout);
    validate_policy_listing(&listing, &engine.parent().unwrap().join("policy.xml")).unwrap();
    let mut command = super::tests::inspect(&engine);
    let output = command
        .env("MAGICK_CONFIGURE_PATH", foreign.path())
        .args(["png:-", "rgb:-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = output;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(include_bytes!("../tests/fixtures/seed.png"))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let detail = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() && output.stdout.is_empty() && detail.contains("security policy"),
        "{detail}"
    );
}

#[test]
#[ignore = "Explicit staged large-image profile only; IMBC_TEST_SIDE and IMBC_TEST_FORMAT required"]
fn security_large_image_profile() {
    let side: u64 = std::env::var("IMBC_TEST_SIDE").unwrap().parse().unwrap();
    assert!(side <= 20000);
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let root = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    let input = root.path().join("large.png");
    if let Some(path) = std::env::var_os("IMBC_TEST_LARGE_SOURCE") {
        let path = PathBuf::from(path);
        assert!(path.is_absolute() && path.is_file());
        fs::copy(path, &input).unwrap();
    } else {
        let job = Job::new().unwrap();
        JOB_IMAGE.with(|v| v.set((side * side, 8, 4)));
        let mut seed = job_file().unwrap();
        seed.write_all(include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"))
            .unwrap();
        seed.seek(SeekFrom::Start(0)).unwrap();
        let out = File::create(&input).unwrap();
        let geometry = format!("{side}x{side}!");
        let mut args = vec!["png:-", "-resize", &geometry, "-depth", "8"];
        if std::env::var_os("IMBC_TEST_ALPHA").is_some() {
            args.extend([
                "-alpha",
                "on",
                "-channel",
                "A",
                "-evaluate",
                "Set",
                "50%",
                "+channel",
            ]);
        }
        args.push("png:-");
        let result = run(&engine, &args, &seed, &out, &AtomicBool::new(false));
        drop((seed, out));
        job.finish(result).unwrap();
    }
    let format = match std::env::var("IMBC_TEST_FORMAT").unwrap().as_str() {
        "avif" => Format::Avif,
        "jxl" => Format::Jxl,
        "webp" => Format::Webp,
        _ => panic!("unsupported test format"),
    };
    let mut source = File::open(&input).unwrap();
    let source_hash = sha256(&mut source).unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut header = [0u8; 24];
    source.read_exact(&mut header).unwrap();
    assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        u32::from_be_bytes(header[16..20].try_into().unwrap()) as u64,
        side
    );
    assert_eq!(
        u32::from_be_bytes(header[20..24].try_into().unwrap()) as u64,
        side
    );
    eprintln!("LARGE_SOURCE side={side} sha256={source_hash}");
    let started = Instant::now();
    TEST_NATIVE_PEAK.with(|v| v.set(0));
    let result = convert_with_options(
        &engine,
        &input,
        root.path(),
        format,
        &Options {
            webp_lossless: true,
            avif_quality: 65,
            ..Default::default()
        },
        &AtomicBool::new(false),
    );
    eprintln!(
        "LARGE side={side} format={format:?} elapsed={:?} result={result:?}",
        started.elapsed()
    );
    let (output, verification) = result.unwrap();
    assert!(output.is_file());
    assert_eq!(verification.input_depth, 8);
    if !matches!(format, Format::Avif) {
        assert!(verification.exact, "{verification:?}");
    }
    assert!(verification.icc && verification.color_verified);
    eprintln!(
        "LARGE_NATIVE observed_peak_bytes={}",
        TEST_NATIVE_PEAK.with(|v| v.get())
    );
    assert_eq!(
        sha256(&mut File::open(&input).unwrap()).unwrap(),
        source_hash
    );
    eprintln!(
        "LARGE_OUTPUT sha256={}",
        sha256(&mut File::open(output).unwrap()).unwrap()
    );
}

#[test]
fn security_source_copy_checks_shared_allowance_before_writing() {
    let job = Job::new().unwrap();
    JOB_BUDGET.with(|b| {
        *b.borrow_mut() = Some(std::sync::Arc::new(engine_limits::Budget::with_limit(8)))
    });
    let mut source = job_file().unwrap();
    source.write_all(b"unchanged original").unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut output = job_file().unwrap();
    let error = copy_bytes(&mut source, &mut output, 18, &AtomicBool::new(false)).unwrap_err();
    assert!(error.contains("上限"));
    assert_eq!(output.metadata().unwrap().len(), 0);
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"unchanged original");
    drop((source, output));
    job.finish(Ok(())).unwrap();
}

#[test]
#[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
fn security_real_engine_output_limit_prevents_publication() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source.png");
    let job = Job::new().unwrap();
    let mut seed = job_file().unwrap();
    seed.write_all(include_bytes!("../tests/fixtures/seed.png"))
        .unwrap();
    seed.seek(SeekFrom::Start(0)).unwrap();
    let generated = File::create(&input).unwrap();
    run(
        &engine,
        &["png:-", "-resize", "100x100!", "png:-"],
        &seed,
        &generated,
        &AtomicBool::new(false),
    )
    .unwrap();
    drop((seed, generated));
    job.finish(Ok(())).unwrap();
    let output = root.path().join("output");
    fs::create_dir(&output).unwrap();
    let before = fs::read(&input).unwrap();
    // policy 清單及 identify 可通過；100x100 的浮點 MIFF 中繼超過此額度。
    engine_limits::TEST_OUTPUT_LIMIT.with(|n| n.set(4096));
    let result = convert(
        &engine,
        &input,
        &output,
        Format::Png,
        &AtomicBool::new(false),
    );
    engine_limits::TEST_OUTPUT_LIMIT.with(|n| n.set(engine_limits::OUTPUT_LIMIT));
    assert!(result.unwrap_err().contains("上限"));
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
    assert_eq!(fs::read(input).unwrap(), before);
}

#[cfg(windows)]
#[test]
fn security_stderr_flood_stops_child_and_cleans_owned_scratch() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let input = job_file().unwrap();
    let output = job_file().unwrap();
    let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let start = Instant::now();
    let result = run(
        &shell,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$s='x'*65536; while ($true) { [Console]::Error.Write($s); [Console]::Error.Flush() }",
        ],
        &input,
        &output,
        &AtomicBool::new(false),
    );
    assert!(result.unwrap_err().contains("上限"));
    assert!(start.elapsed() < Duration::from_secs(10));
    drop((input, output));
    job.finish(Ok(())).unwrap();
    assert!(!root.exists());
}

#[cfg(windows)]
#[test]
fn security_cancellation_with_stderr_keeps_exact_cancel_result() {
    let job = Job::new().unwrap();
    let input = job_file().unwrap();
    let output = job_file().unwrap();
    let cancel = AtomicBool::new(false);
    let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let start = Instant::now();
            while output.metadata().unwrap().len() == 0 && start.elapsed() < Duration::from_secs(8)
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            cancel.store(true, Ordering::Relaxed);
        });
        run(
            &shell,
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Error.WriteLine('diagnostic'); [Console]::Out.Write('ready'); [Console]::Out.Flush(); Start-Sleep -Seconds 30",
            ],
            &input,
            &output,
            &cancel,
        )
    });
    assert_eq!(result.unwrap_err(), "已取消");
    assert_eq!(output.metadata().unwrap().len(), 5);
    drop((input, output));
    job.finish(Ok(())).unwrap();
}

#[test]
#[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
fn security_real_engine_denies_external_lists_network_coders_and_filters() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let job = Job::new().unwrap();
    let mut source = job_file().unwrap();
    source
        .write_all(include_bytes!("../tests/fixtures/seed.png"))
        .unwrap();
    let sentinel = job_dir().join("external-list.txt");
    fs::write(
        &sentinel,
        b"private sentinel; never interpret as an image list",
    )
    .unwrap();
    let list = format!("@{}", sentinel.display());
    for (args, absent_module) in [
        (vec!["svg:-", "png:-"], None),
        (vec!["mvg:-", "png:-"], Some("mvg")),
        (vec!["pdf:-", "png:-"], None),
        (vec!["https:-", "png:-"], Some("url")),
        (vec!["png:-", "rgb:-"], None),
        (vec!["png:-", "-process", "analyze", "png:-"], None),
        (vec![list.as_str(), "png:-"], None),
    ] {
        source.seek(SeekFrom::Start(0)).unwrap();
        let output = job_file().unwrap();
        let error = run(&engine, &args, &source, &output, &AtomicBool::new(false)).unwrap_err();
        if let Some(module) = absent_module {
            // 隨包沒有這些模組，拒絕原因與 policy 擋下不同，不混稱為 policy 證據。
            assert!(
                !engine
                    .parent()
                    .unwrap()
                    .join(format!("modules/coders/IM_MOD_RL_{module}_.dll"))
                    .exists()
            );
            assert!(error.contains("no decode delegate"), "{args:?}: {error}");
        } else {
            assert!(
                error.contains("security policy") || error.contains("not authorized"),
                "{args:?}: {error}"
            );
        }
        assert_eq!(output.metadata().unwrap().len(), 0, "{args:?}");
    }
    assert_eq!(
        fs::read(sentinel).unwrap(),
        b"private sentinel; never interpret as an image list"
    );
    drop(source);
    job.finish(Ok(())).unwrap();
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

// 2026-11-13 P1-C 子程序：建立殘留後以 TerminateProcess 硬性結束自己（不執行任何解構器）。
#[test]
#[ignore = "Subprocess fixture only; run security_crashed_instance_residue_is_recovered"]
fn crash_instance_fixture() {
    use std::os::windows::{fs::OpenOptionsExt, process::CommandExt};
    let Some(state) = std::env::var_os("IMBC_CRASH_FIXTURE") else {
        return;
    };
    let state = PathBuf::from(state);
    let shell_dir = windows_directory()
        .unwrap()
        .join("System32/WindowsPowerShell/v1.0");
    let ledger = std::sync::Arc::new(
        instance_ledger::Ledger::open_in(&state.join("runtime"), std::slice::from_ref(&shell_dir))
            .unwrap(),
    );
    let work = state.join("imb-c-work-crashed");
    fs::create_dir_all(work.join("imb-c-job-x").join("imb-c-cache-y")).unwrap();
    fs::write(
        work.join("imb-c-job-x/imb-c-cache-y/magick-cache"),
        [7u8; 4096],
    )
    .unwrap();
    let handle = File::options()
        .access_mode(0x80)
        .custom_flags(0x0220_0000)
        .open(&work)
        .unwrap();
    std::mem::forget(
        ledger
            .record(instance_ledger::Kind::Dir, &handle, &work)
            .unwrap(),
    );
    drop(handle);
    let partial = state.join(".tmpPARTIAL");
    let file = File::create(&partial).unwrap();
    std::mem::forget(
        ledger
            .record(instance_ledger::Kind::File, &file, &partial)
            .unwrap(),
    );
    drop(file);
    std::mem::forget(
        ledger
            .try_reserve(instance_ledger::Resource::Memory, 1 << 20, u64::MAX)
            .unwrap()
            .unwrap(),
    );
    // 建立→納管空窗：已暫停建立、尚未納入 Job 時父程序消失。
    let child = std::process::Command::new(shell_dir.join("powershell.exe"))
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 60",
        ])
        .creation_flags(0x0800_0004) // CREATE_NO_WINDOW | CREATE_SUSPENDED
        .spawn()
        .unwrap();
    fs::write(state.join("orphan.pid"), child.id().to_string()).unwrap();
    std::mem::forget(child);
    unsafe { TerminateProcess(GetCurrentProcess(), 9) };
}

#[test]
fn security_crashed_instance_residue_is_recovered() {
    let state = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    // 不屬於已崩潰執行個體的程序：不得被終止。
    let mut bystander = std::process::Command::new(
        windows_directory()
            .unwrap()
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 60",
    ])
    .stdout(std::process::Stdio::null())
    .spawn()
    .unwrap();
    let user_file = state.path().join("user-photo.png");
    fs::write(&user_file, b"original").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["crash_instance_fixture", "--ignored", "--test-threads=1"])
        .env("IMBC_CRASH_FIXTURE", state.path())
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(9));
    let orphan: u32 = fs::read_to_string(state.path().join("orphan.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let orphan_handle = unsafe { OpenProcess(0x0010_0000, 0, orphan) }; // SYNCHRONIZE
    assert!(!orphan_handle.is_null());
    assert_eq!(
        unsafe { WaitForSingleObject(orphan_handle, 0) },
        258,
        "孤兒應仍存活（暫停）"
    );
    assert!(state.path().join("imb-c-work-crashed").is_dir());

    let survivor = std::sync::Arc::new(
        instance_ledger::Ledger::open_in(&state.path().join("runtime"), &[]).unwrap(),
    );
    // 崩潰者的記憶體預留在存取時即被移除。
    assert!(
        survivor
            .try_reserve(instance_ledger::Resource::Memory, 1, 1)
            .unwrap()
            .is_ok()
    );
    let report = survivor.recover().unwrap();
    assert_eq!(report.journals, 1, "{report:?}");
    assert_eq!(report.orphans, 1, "{report:?}");
    assert_eq!(report.removed, 2, "{report:?}");
    assert!(report.kept.is_empty(), "{report:?}");
    assert_eq!(unsafe { WaitForSingleObject(orphan_handle, 5000) }, 0);
    unsafe { CloseHandle(orphan_handle) };
    assert!(!state.path().join("imb-c-work-crashed").exists());
    assert!(!state.path().join(".tmpPARTIAL").exists());
    assert_eq!(fs::read(&user_file).unwrap(), b"original");
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "無關程序不得被終止"
    );
    bystander.kill().unwrap();
    bystander.wait().unwrap();
}

// 2026-11-13 P1-B 多程序回歸：子程序各自是獨立執行個體，競爭同一份預留表。
#[test]
#[ignore = "Subprocess fixture only; run security_ledger_reservations_hold_across_processes"]
fn ledger_contender_fixture() {
    let Some(state) = std::env::var_os("IMBC_LEDGER_FIXTURE") else {
        return;
    };
    let state = PathBuf::from(state);
    let name = std::env::var("IMBC_LEDGER_NAME").unwrap();
    let ledger =
        std::sync::Arc::new(instance_ledger::Ledger::open_in(&state.join("runtime"), &[]).unwrap());
    let granted = ledger
        .try_reserve(instance_ledger::Resource::Memory, 30, 100)
        .unwrap()
        .ok();
    fs::write(
        state.join(format!("{name}.result")),
        if granted.is_some() {
            "granted"
        } else {
            "denied"
        },
    )
    .unwrap();
    let started = Instant::now();
    while !state.join("release").exists() {
        assert!(started.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(granted);
}

#[test]
fn security_ledger_reservations_hold_across_processes() {
    let state = tempfile::tempdir().unwrap();
    let mut children: Vec<_> = (0..6)
        .map(|i| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["ledger_contender_fixture", "--ignored", "--test-threads=1"])
                .env("IMBC_LEDGER_FIXTURE", state.path())
                .env("IMBC_LEDGER_NAME", format!("c{i}"))
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let started = Instant::now();
    let results: Vec<String> = (0..6)
        .map(|i| {
            loop {
                if let Ok(text) = fs::read_to_string(state.path().join(format!("c{i}.result")))
                    && !text.is_empty()
                {
                    break text;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "child {i} did not report"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        })
        .collect();
    // 容量 100、每份 30：同時存活的程序最多核准 3 份，不得重複計算同一份容量。
    let granted: Vec<usize> = (0..6).filter(|i| results[*i] == "granted").collect();
    assert_eq!(granted.len(), 3, "{results:?}");
    let observer = std::sync::Arc::new(
        instance_ledger::Ledger::open_in(&state.path().join("runtime"), &[]).unwrap(),
    );
    assert_eq!(
        observer
            .try_reserve(instance_ledger::Resource::Memory, 30, 100)
            .unwrap()
            .err(),
        Some(90)
    );
    // 一個持有預留的程序被強制終止（不執行解構器）：其預留由 OS 釋放存活鎖後被回收。
    children[granted[0]].kill().unwrap();
    children[granted[0]].wait().unwrap();
    let reclaimed = observer
        .try_reserve(instance_ledger::Resource::Memory, 30, 100)
        .unwrap()
        .expect("killed holder's reservation must be reclaimed");
    drop(reclaimed);
    fs::write(state.path().join("release"), b"go").unwrap();
    for (i, child) in children.iter_mut().enumerate() {
        if i != granted[0] {
            assert!(child.wait().unwrap().success());
        }
    }
    // 全部正常釋放後，完整容量可再次預留。
    assert!(
        observer
            .try_reserve(instance_ledger::Resource::Memory, 100, 100)
            .unwrap()
            .is_ok()
    );
}

// 手動維護：回收已結束測試程序在測試專用狀態目錄留下的紀錄與保留暫存（不碰使用者正式狀態）。
#[test]
#[ignore = "Manual maintenance of the test-only runtime ledger"]
fn security_recover_finished_test_instances() {
    let ledger = instance_ledger::global().unwrap();
    let report = ledger.recover().unwrap();
    println!("RECOVERY {report:?}");
    assert!(report.kept.is_empty(), "{report:?}");
}

// Low IL 只限制可寫位置；授權暫存內仍須由核心 Job I/O 監控停止超量寫入。
// 這是寫入後的累計防線，不是磁碟硬配額，也沒有嚴格的最大超出量保證。
#[test]
fn security_native_writes_inside_low_dir_hit_job_write_ceiling() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let input = job_file().unwrap();
    let output = job_file().unwrap();
    let shell = windows_directory()
        .unwrap()
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = "$ErrorActionPreference='Stop'; $f=[IO.File]::Open([IO.Path]::Combine($env:TEMP,'bounded.bin'),'CreateNew'); $b=New-Object byte[] 1048576; for($i=0;$i -lt 256;$i++){ $f.Write($b,0,$b.Length); $f.Flush(); Start-Sleep -Milliseconds 20 }; $f.Close()";
    engine_limits::TEST_NATIVE_WRITE_LIMIT.with(|v| v.set(4 * 1024 * 1024));
    let started = Instant::now();
    let result = run(
        &shell,
        &["-NoProfile", "-NonInteractive", "-Command", script],
        &input,
        &output,
        &AtomicBool::new(false),
    );
    engine_limits::TEST_NATIVE_WRITE_LIMIT.with(|v| v.set(engine_limits::NATIVE_WRITE_LIMIT));
    let error = result.unwrap_err();
    assert!(error.contains("累計寫入"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(output.metadata().unwrap().len(), 0);
    drop((input, output));
    assert!(job.finish::<()>(Err(error)).is_err());
    assert!(!root.exists());
}

// 2026-11-13：libjxl 0.13.0／Brotli 1.2.1 安全更新後的 JPEG ↔ JXL 跨版本無損還原。
// s444-q95.jpg 是 4:4:4 JPEG；cjxl 0.13 預設 LF smoothing 產生的 JXL 交給 djxl 0.12
// 會以退出碼 0 還原出不同位元組（work/claude-round2-20261113/compat 實測 9/23）。
#[test]
#[ignore = "Requires v1.3 runtime with JXL tools; set IMBC_TEST_ENGINE"]
fn security_jxl_013_keeps_cross_version_jpeg_reconstruction() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let tools = engine.parent().unwrap().parent().unwrap().join("JXL");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jxl-compat");
    let jpeg = fixtures.join("s444-q95.jpg");
    let original = fs::read(&jpeg).unwrap();
    let off = AtomicBool::new(false);
    // 舊版 cjxl 0.12.0 產生的 JXL：新版 djxl 必須逐位元組還原。
    let out = tempfile::tempdir().unwrap();
    let old = out.path().join("legacy.jxl");
    fs::copy(fixtures.join("s444-q95.cjxl-0.12.0.jxl"), &old).unwrap();
    let (restored, _, verified) =
        jpeg_reconstruction(&tools, &old, out.path(), false, Some(&jpeg), &off).unwrap();
    assert!(verified);
    assert_eq!(fs::read(restored).unwrap(), original);
    // 新封裝必須關閉 LF smoothing：與 0.13.0 加 --jpeg_reconstruction_lfs=0 的位元流相同，
    // 這個位元流已實測可由 djxl 0.12.0 逐位元組還原。
    let out = tempfile::tempdir().unwrap();
    let (packed, hash, verified) =
        jpeg_reconstruction(&tools, &jpeg, out.path(), true, None, &off).unwrap();
    assert!(verified);
    assert_eq!(hash, sha256(&mut File::open(&jpeg).unwrap()).unwrap());
    assert_eq!(
        fs::read(&packed).unwrap(),
        fs::read(fixtures.join("s444-q95.cjxl-0.13.0-lfs0.jxl")).unwrap()
    );
    let back = tempfile::tempdir().unwrap();
    let (restored, _, verified) =
        jpeg_reconstruction(&tools, &packed, back.path(), false, Some(&jpeg), &off).unwrap();
    assert!(verified);
    assert_eq!(fs::read(restored).unwrap(), original);
}
