//! RC2 第三輪：快取回收所有權、預留原子升級、目的端暫存刪除失敗與部分複製後取消。
//! 測試只在自己建立的暫存目錄內讀寫；不觸碰使用者資料、私人素材或發布封包。
use super::*;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::Arc,
};

fn lower(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(tree(&path));
        } else {
            out.push((lower(&path), fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

// ───────────── 子程序 fixture ─────────────

/// 以獨立程序執行快取與帳本動作；由 `IMBC_RC3_MODE` 指定。
#[test]
#[ignore = "Subprocess fixture only"]
fn rc3_process_fixture() {
    let mode = std::env::var("IMBC_RC3_MODE").unwrap();
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).unwrap();
    let mut out = std::io::stdout();
    let wait_for_parent = || {
        let mut rest = String::new();
        let _ = std::io::stdin().read_to_string(&mut rest);
    };
    match mode.as_str() {
        // 建立（或與其他程序競爭建立）並使用快取：讀取內容後持有租約直到 stdin 關閉。
        "hold" => {
            let root = var("IMBC_RC3_ROOT");
            let source = var("IMBC_RC3_SOURCE");
            let key = std::env::var("IMBC_RC3_KEY").unwrap();
            let (dir, pins) = runtime_cache::test_support::prepare_at(
                &root,
                &key,
                &[("JXL/cjxl.exe", &source, None)],
            )
            .unwrap();
            let used =
                fs::read(dir.join("JXL").join("cjxl.exe")).unwrap() == fs::read(&source).unwrap();
            writeln!(out, "\nREADY used={used} {}", dir.display()).unwrap();
            out.flush().unwrap();
            wait_for_parent();
            drop(pins);
        }
        // 另一個版本啟動：觸發該根目錄的回收。
        "clean" => {
            let root = var("IMBC_RC3_ROOT");
            let source = var("IMBC_RC3_SOURCE");
            let key = std::env::var("IMBC_RC3_KEY").unwrap();
            runtime_cache::test_support::reset_cleanup(&root);
            let (_, pins) = runtime_cache::test_support::prepare_at(
                &root,
                &key,
                &[("JXL/cjxl.exe", &source, None)],
            )
            .unwrap();
            writeln!(out, "\nDONE").unwrap();
            out.flush().unwrap();
            drop(pins);
        }
        // 早期預留後原子升級；持有預留直到 stdin 關閉。
        "reserve" => {
            let ledger =
                Arc::new(instance_ledger::Ledger::open_in(&var("IMBC_RC3_LEDGER"), &[]).unwrap());
            let number = |name: &str| std::env::var(name).unwrap().parse::<u64>().unwrap();
            let (early, target, capacity) = (
                number("IMBC_RC3_EARLY"),
                number("IMBC_RC3_TARGET"),
                number("IMBC_RC3_CAPACITY"),
            );
            let resource = instance_ledger::Resource::Disk("rc3-race");
            let held = ledger
                .try_reserve(resource, early, capacity)
                .unwrap()
                .unwrap();
            // 兩個程序在父程序釋放後同時升級。
            writeln!(out, "\nARMED").unwrap();
            out.flush().unwrap();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            let result = ledger
                .reserve_many(
                    Some(&held),
                    &[(
                        instance_ledger::Resource::Disk("rc3-race"),
                        target,
                        capacity,
                    )],
                )
                .unwrap();
            writeln!(
                out,
                "\nRESULT {}",
                if result.is_ok() { "ok" } else { "err" }
            )
            .unwrap();
            out.flush().unwrap();
            wait_for_parent();
            drop(held);
        }
        // 交付途中取消且暫存刪除失敗，之後不經正常關閉就結束（模擬中斷）。
        "orphan" => {
            let destination_path = var("IMBC_RC3_DEST");
            let job = Job::new().unwrap();
            let destination = Destination::open(&destination_path).unwrap();
            let mut local = job_file().unwrap();
            local.write_all(&vec![3u8; 4 << 20]).unwrap();
            static CANCEL: AtomicBool = AtomicBool::new(false);
            TEST_COPY_HOOK.with(|h| {
                *h.borrow_mut() = Some(Box::new(|_| CANCEL.store(true, Ordering::Relaxed)))
            });
            destination::TEST_FAIL_DISCARD.with(|v| v.set(true));
            let error = deliver(
                &mut local,
                &destination,
                std::ffi::OsStr::new("orphan"),
                Format::Png,
                &CANCEL,
            )
            .unwrap_err();
            writeln!(out, "\nERROR {error}").unwrap();
            out.flush().unwrap();
            std::mem::forget(job);
            std::process::exit(0);
        }
        other => panic!("未知 rc3 fixture 模式 {other}"),
    }
}

fn fixture(mode: &str, envs: &[(&str, &std::ffi::OsStr)]) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "engine::regression_rc3::rc3_process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("IMBC_RC3_MODE", mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in envs {
        command.env(k, v);
    }
    command.spawn().unwrap()
}
/// 讀到以 `prefix` 開頭的行為止（忽略測試框架輸出）。
fn read_marker(child: &mut Child, prefix: &str) -> String {
    let stdout = child.stdout.as_mut().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            reader.read_line(&mut line).unwrap(),
            0,
            "子程序未輸出 {prefix}"
        );
        if let Some(rest) = line.trim_end().strip_prefix(prefix) {
            return rest.trim().to_owned();
        }
    }
}
fn finish(mut child: Child) {
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

// ───────────── 1. 快取回收：預設拒絕 ─────────────

/// 未知目錄、被替換的登錄目錄、重新解析點（junction）與其中的檔案，回收後完整保留；
/// 只有登錄且身分相符、沒有租約的舊版快取被移除。
#[test]
fn rc3_cleanup_preserves_unknown_replaced_and_reparse_directories() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("cjxl.exe");
    fs::write(&source, b"engine v1").unwrap();
    let root = work.path().join("cache");
    let files = [("JXL/cjxl.exe", source.as_path(), None)];
    // 登錄的舊版快取（應移除）與之後被替換的登錄快取。
    let (old, pins) = prepare_at(&root, "1.0.0-old", &files).unwrap();
    drop(pins);
    let (victim, pins) = prepare_at(&root, "1.0.0-victim", &files).unwrap();
    drop(pins);
    fs::rename(&victim, root.join("victim-moved-by-user")).unwrap();
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("使用者資料.txt"), b"user data in replaced dir").unwrap();
    // 未登錄、名稱像快取的目錄，以及一般使用者目錄。
    for name in [
        "0.9.0-unregistered",
        "staging-unknown",
        "trash-unknown",
        "使用者 資料夾",
    ] {
        fs::create_dir_all(root.join(name).join("JXL")).unwrap();
        fs::write(
            root.join(name).join("JXL").join("cjxl.exe"),
            name.as_bytes(),
        )
        .unwrap();
        fs::write(root.join(name).join("note.txt"), b"keep me").unwrap();
    }
    // 指向快取根目錄外的 junction（不需系統管理員權限）。
    let outside = work.path().join("outside target");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("precious.txt"), b"outside data").unwrap();
    let status = Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(root.join("1.0.0-junction"))
        .arg(&outside)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let before = (
        tree(&root.join("victim-moved-by-user")),
        tree(&victim),
        tree(&outside),
    );
    let unknown_before: Vec<_> = [
        "0.9.0-unregistered",
        "staging-unknown",
        "trash-unknown",
        "使用者 資料夾",
    ]
    .iter()
    .map(|n| tree(&root.join(n)))
    .collect();
    reset_cleanup(&root);
    let (current, pins) = prepare_at(&root, "2.0.0-current", &files).unwrap();
    assert!(!old.exists(), "登錄且無租約的舊版快取應移除");
    assert!(current.exists());
    assert_eq!(
        (
            tree(&root.join("victim-moved-by-user")),
            tree(&victim),
            tree(&outside)
        ),
        before,
        "被替換的目錄、被移走的原快取與 junction 目標都必須完整保留"
    );
    for (name, expected) in [
        "0.9.0-unregistered",
        "staging-unknown",
        "trash-unknown",
        "使用者 資料夾",
    ]
    .iter()
    .zip(&unknown_before)
    {
        assert_eq!(&tree(&root.join(name)), expected, "{name}");
    }
    assert!(root.join("1.0.0-junction").exists());
    drop(pins);
    // 清理測試目錄：先移除 junction 本身（不跟隨）。
    fs::remove_dir(root.join("1.0.0-junction")).unwrap();
}

/// 被植入清單外檔案的目前版本快取：清單內項目刪除、植入檔完整保留在 quarantine-*，快取重建。
#[test]
fn rc3_invalid_cache_with_unknown_content_is_quarantined_not_deleted() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("cjxl.exe");
    fs::write(&source, b"engine").unwrap();
    let root = work.path().join("cache");
    let files = [("JXL/cjxl.exe", source.as_path(), None)];
    let (key_dir, pins) = prepare_at(&root, "1.0.0-q", &files).unwrap();
    drop(pins);
    let user = current_user();
    set_tree_sddl(
        &key_dir.join("JXL"),
        &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"),
    );
    fs::write(key_dir.join("JXL").join("planted.txt"), b"planted by owner").unwrap();
    let (rebuilt, pins) = prepare_at(&root, "1.0.0-q", &files).unwrap();
    assert_eq!(
        fs::read(rebuilt.join("JXL").join("cjxl.exe")).unwrap(),
        b"engine"
    );
    drop(pins);
    let quarantine: Vec<_> = names(&root)
        .into_iter()
        .filter(|n| n.starts_with("quarantine-"))
        .collect();
    assert_eq!(quarantine.len(), 1, "{:?}", names(&root));
    let kept = root.join(&quarantine[0]).join("JXL").join("planted.txt");
    assert_eq!(fs::read(&kept).unwrap(), b"planted by owner");
    set_tree_sddl(
        &root.join(&quarantine[0]),
        &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"),
    );
}

#[test]
fn rc3_cache_recovery_preserves_same_name_replacement_and_quarantine() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("cjxl.exe");
    fs::write(&source, b"engine").unwrap();
    let root = work.path().join("cache");
    let entries = [("JXL/cjxl.exe", source.as_path(), None)];
    let key = "1.0.0-replacement";
    let (version, pins) = prepare_at(&root, key, &entries).unwrap();
    drop(pins);
    let dir = version.join("JXL");
    let cached = dir.join("cjxl.exe");
    let original_acl = sddl_of(&cached);
    let original_dir_acl = sddl_of(&dir);
    let user = current_user();
    set_tree_sddl(&dir, &format!("D:P(A;;FA;;;{user})(A;;FA;;;SY)"));
    let replacement = dir.join("replacement");
    fs::write(&replacement, b"preserve this replacement").unwrap();
    fs::remove_file(&cached).unwrap();
    fs::rename(&replacement, &cached).unwrap();
    set_tree_sddl(&dir, &original_dir_acl);
    set_tree_sddl(&cached, &original_acl);

    let (_, pins) = prepare_at(&root, key, &entries).unwrap();
    let quarantine = names(&root)
        .into_iter()
        .find(|name| name.starts_with("quarantine-"))
        .unwrap();
    let kept = root.join(quarantine).join("JXL").join("cjxl.exe");
    assert_eq!(fs::read(&kept).unwrap(), b"preserve this replacement");
    reset_cleanup(&root);
    let _next = prepare_at(&root, "1.0.1-next", &entries).unwrap();
    assert_eq!(fs::read(&kept).unwrap(), b"preserve this replacement");
    drop(pins);
}

// ───────────── 2. 兩個獨立程序：建立、使用、回收與租約 ─────────────

#[test]
fn rc3_two_processes_build_and_use_cache_and_leases_block_reclaim() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("cjxl.exe");
    fs::write(&source, vec![7u8; 8 << 20]).unwrap();
    let root = work.path().join("cache 根");
    fs::create_dir(&root).unwrap();
    let unknown = root.join("未知 目錄");
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("data.bin"), b"unknown").unwrap();
    let envs = |key: &str| {
        vec![
            ("IMBC_RC3_ROOT", root.as_os_str().to_owned()),
            ("IMBC_RC3_SOURCE", source.as_os_str().to_owned()),
            ("IMBC_RC3_KEY", std::ffi::OsString::from(key)),
        ]
    };
    let spawn = |mode: &str, key: &str| {
        let owned = envs(key);
        let refs: Vec<_> = owned.iter().map(|(k, v)| (*k, v.as_os_str())).collect();
        fixture(mode, &refs)
    };
    // 兩個程序同時建立並使用同一版本快取。
    let mut first = spawn("hold", "1.0.0-shared");
    let mut second = spawn("hold", "1.0.0-shared");
    let a = read_marker(&mut first, "READY");
    let b = read_marker(&mut second, "READY");
    assert!(
        a.starts_with("used=true") && b.starts_with("used=true"),
        "{a} / {b}"
    );
    assert_eq!(lower(Path::new(&a[10..])), lower(Path::new(&b[10..])));
    let shared = root.join("1.0.0-shared");
    assert!(
        !names(&root).iter().any(|n| n.starts_with("staging-")),
        "{:?}",
        names(&root)
    );
    // 另一個版本在租約期間啟動並回收：被持有的快取必須保留。
    let mut cleaner = spawn("clean", "2.0.0-next");
    read_marker(&mut cleaner, "DONE");
    finish(cleaner);
    assert_eq!(
        fs::read(shared.join("JXL").join("cjxl.exe")).unwrap(),
        fs::read(&source).unwrap()
    );
    // 只剩一個程序持有時仍保留。
    finish(first);
    let mut cleaner = spawn("clean", "2.0.0-next");
    read_marker(&mut cleaner, "DONE");
    finish(cleaner);
    assert!(shared.exists(), "仍有租約時不得刪除");
    // 全部釋放後才回收；未知目錄始終完整保留。
    finish(second);
    let mut cleaner = spawn("clean", "2.0.0-next");
    read_marker(&mut cleaner, "DONE");
    finish(cleaner);
    assert!(!shared.exists(), "租約全部釋放後應回收：{:?}", names(&root));
    assert_eq!(fs::read(unknown.join("data.bin")).unwrap(), b"unknown");
}

// ───────────── 3. 預留原子升級 ─────────────

#[test]
fn rc3_reservation_upgrade_excludes_own_early_amount_at_capacity_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(instance_ledger::Ledger::open_in(dir.path(), &[]).unwrap());
    let other = Arc::new(instance_ledger::Ledger::open_in(dir.path(), &[]).unwrap());
    let disk = || instance_ledger::Resource::Disk("rc3-boundary");
    let early = ledger.try_reserve(disk(), 60, 100).unwrap().unwrap();
    // 舊做法會把 60 + 100 一起計入而拒絕；現在剛好等於容量即可升級。
    assert!(
        ledger
            .reserve_many(Some(&early), &[(disk(), 100, 100)])
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert_eq!(ledger.reserved_bytes(&early).unwrap(), Some(100));
    assert_eq!(ledger.held_bytes(disk()).unwrap(), 100, "同一工作只計一次");
    // 超過容量 1 byte：拒絕且原預留不變。
    let refused = ledger
        .reserve_many(Some(&early), &[(disk(), 101, 100)])
        .unwrap();
    assert_eq!(refused.err().unwrap(), (0, 0));
    assert_eq!(ledger.reserved_bytes(&early).unwrap(), Some(100));
    // 其他執行個體持有 30：降回 70 可行，71 拒絕。
    let shrink = ledger
        .reserve_many(Some(&early), &[(disk(), 20, 100)])
        .unwrap();
    assert!(shrink.is_ok());
    let held = other.try_reserve(disk(), 30, 100).unwrap().unwrap();
    assert!(
        ledger
            .reserve_many(Some(&early), &[(disk(), 70, 100)])
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        ledger
            .reserve_many(Some(&early), &[(disk(), 71, 100)])
            .unwrap()
            .err()
            .unwrap(),
        (0, 30)
    );
    assert_eq!(ledger.held_bytes(disk()).unwrap(), 100);
    // 不同磁碟區：升級暫存並新增目的端，任一失敗全部不變。
    let out = || instance_ledger::Resource::Disk("rc3-output");
    let refused = ledger
        .reserve_many(Some(&early), &[(disk(), 70, 100), (out(), 51, 50)])
        .unwrap();
    assert_eq!(refused.err().unwrap(), (1, 0));
    assert_eq!(ledger.held_bytes(out()).unwrap(), 0);
    let added = ledger
        .reserve_many(Some(&early), &[(disk(), 70, 100), (out(), 50, 50)])
        .unwrap()
        .unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(ledger.held_bytes(out()).unwrap(), 50);
    drop((added, early, held));
    assert_eq!(ledger.held_bytes(disk()).unwrap(), 0);
}

/// 兩個獨立程序各持有 40，同時升級到 60（容量 100）：只能有一個成功，總量不超過容量。
#[test]
fn rc3_reservation_upgrade_race_between_processes_never_exceeds_capacity() {
    let dir = tempfile::tempdir().unwrap();
    for round in 0..5 {
        let envs = [
            ("IMBC_RC3_LEDGER", dir.path().as_os_str()),
            ("IMBC_RC3_EARLY", std::ffi::OsStr::new("40")),
            ("IMBC_RC3_TARGET", std::ffi::OsStr::new("60")),
            ("IMBC_RC3_CAPACITY", std::ffi::OsStr::new("100")),
        ];
        let mut a = fixture("reserve", &envs);
        let mut b = fixture("reserve", &envs);
        read_marker(&mut a, "ARMED");
        read_marker(&mut b, "ARMED");
        for child in [&mut a, &mut b] {
            writeln!(child.stdin.as_mut().unwrap(), "go").unwrap();
        }
        let results = [read_marker(&mut a, "RESULT"), read_marker(&mut b, "RESULT")];
        let observer = Arc::new(instance_ledger::Ledger::open_in(dir.path(), &[]).unwrap());
        let total = observer
            .held_bytes(instance_ledger::Resource::Disk("rc3-race"))
            .unwrap();
        assert_eq!(
            results.iter().filter(|r| *r == "ok").count(),
            1,
            "round {round}: {results:?}"
        );
        assert_eq!(total, 100, "round {round}");
        finish(a);
        finish(b);
    }
}

// ───────────── 4. 部分複製後取消、目的端刪除失敗 ─────────────

static PARTIAL_CANCEL: AtomicBool = AtomicBool::new(false);
struct HookReset;
impl Drop for HookReset {
    fn drop(&mut self) {
        TEST_COPY_HOOK.with(|h| *h.borrow_mut() = None);
        destination::TEST_FAIL_DISCARD.with(|v| v.set(false));
        filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = None);
        PARTIAL_CANCEL.store(false, Ordering::Relaxed);
    }
}
fn cancel_after_first_chunk() -> HookReset {
    PARTIAL_CANCEL.store(false, Ordering::Relaxed);
    TEST_COPY_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(|_| PARTIAL_CANCEL.store(true, Ordering::Relaxed)))
    });
    HookReset
}

fn partial_cancel_case(destination_root: &Path) {
    let job = Job::new().unwrap();
    let out = tempfile::Builder::new()
        .prefix("imb-c-rc3-out-")
        .tempdir_in(destination_root)
        .unwrap();
    fs::write(out.path().join("image.png"), b"existing").unwrap();
    let destination = Destination::open(out.path()).unwrap();
    let mut local = job_file().unwrap();
    local.write_all(&vec![5u8; 8 << 20]).unwrap();
    {
        let _reset = cancel_after_first_chunk();
        let error = deliver(
            &mut local,
            &destination,
            std::ffi::OsStr::new("image"),
            Format::Png,
            &PARTIAL_CANCEL,
        )
        .unwrap_err();
        assert_eq!(error, "已取消");
    }
    assert_eq!(
        names(out.path()),
        ["image.png"],
        "不得發布不完整輸出或留下暫存"
    );
    assert_eq!(fs::read(out.path().join("image.png")).unwrap(), b"existing");
    // 非本機 NTFS 來源複製到一半取消：精確「已取消」。
    let source = out.path().join("source.bin");
    fs::write(&source, vec![1u8; 4 << 20]).unwrap();
    {
        let _reset = cancel_after_first_chunk();
        filesystem_safety::TEST_CAPABILITIES
            .with(|v| *v.borrow_mut() = Some(("exFAT", 0, false, true)));
        let error = isolated_source(File::open(&source).unwrap(), &PARTIAL_CANCEL)
            .err()
            .unwrap();
        assert_eq!(error, "已取消");
    }
    drop((local, destination));
    job.finish(Ok(())).unwrap();
}

#[test]
fn rc3_cancel_after_partial_copy_never_publishes_local() {
    partial_cancel_case(&controlled_temp::preferred_root().unwrap());
}

/// 目的端暫存刪除失敗：回報錯誤、不發布、保留擁有權紀錄；程序異常結束後，
/// 下一個執行個體以 File ID 核對並回收該暫存。
fn delete_failure_case(destination_root: &Path) {
    let out = tempfile::Builder::new()
        .prefix("imb-c-rc3-delete-")
        .tempdir_in(destination_root)
        .unwrap();
    let mut child = fixture("orphan", &[("IMBC_RC3_DEST", out.path().as_os_str())]);
    let error = read_marker(&mut child, "ERROR");
    assert!(child.wait().unwrap().success());
    assert!(error.starts_with("已取消；輸出暫存"), "{error}");
    assert!(
        error.contains("無法刪除") && error.contains("未發布"),
        "{error}"
    );
    let leftovers: Vec<_> = names(out.path())
        .into_iter()
        .filter(|n| n.starts_with(".tmp"))
        .collect();
    assert_eq!(
        leftovers.len(),
        1,
        "刪除失敗的暫存應保留供回收：{:?}",
        names(out.path())
    );
    assert!(
        !names(out.path()).iter().any(|n| n.starts_with("orphan")),
        "不得發布"
    );
    if error.contains("已保留其擁有權紀錄") {
        let report = instance_ledger::global().unwrap().recover().unwrap();
        assert!(report.removed >= 1, "{report:?}");
        assert!(
            names(out.path()).iter().all(|n| !n.starts_with(".tmp")),
            "下次啟動應回收：{:?}",
            names(out.path())
        );
        println!("PASS recovered by File ID: {error}");
    } else {
        assert!(error.contains("無法自動回收"), "{error}");
        println!("PASS reported without reliable File ID: {error}");
        for name in leftovers {
            fs::remove_file(out.path().join(name)).unwrap();
        }
    }
}

#[test]
fn rc3_destination_delete_failure_is_reported_and_recovered_local() {
    delete_failure_case(&controlled_temp::preferred_root().unwrap());
}

#[test]
#[ignore = "SMB destination; set IMBC_TEST_UNC_ROOT"]
fn rc3_smb_partial_cancel_and_delete_failure_recovery() {
    let root = PathBuf::from(std::env::var_os("IMBC_TEST_UNC_ROOT").unwrap());
    partial_cancel_case(&root);
    delete_failure_case(&root);
}
