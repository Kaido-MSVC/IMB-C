//! v1.4 格式回歸，實際引擎與外部驗證皆可選用。
use super::*;
use std::fs;

#[test]
#[ignore = "Explicit bounded 16 MP regression; set IMBC_TEST_ENGINE"]
fn v14_large_formats() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let root = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    let input = root.path().join("large.png");
    let off = AtomicBool::new(false);
    let job = Job::new().unwrap();
    JOB_IMAGE.with(|v| v.set((16_000_000, 8, 3)));
    let mut seed = job_file().unwrap();
    use std::io::Write;
    seed.write_all(include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"))
        .unwrap();
    seed.seek(SeekFrom::Start(0)).unwrap();
    let output = File::create(&input).unwrap();
    let result = run(
        &engine,
        &[
            "png:-",
            "-resize",
            "4000x4000!",
            "-depth",
            "8",
            "-alpha",
            "off",
            "png:-",
        ],
        &seed,
        &output,
        &off,
    );
    drop((seed, output));
    job.finish(result).unwrap();
    let before = sha256(&mut File::open(&input).unwrap()).unwrap();
    for (format, options) in [
        (
            Format::Avif,
            Options {
                avif_lossless: true,
                ..Default::default()
            },
        ),
        (
            Format::Jxl,
            Options {
                jxl_lossless: false,
                jxl_quality: 75,
                ..Default::default()
            },
        ),
        (Format::Psb, Options::default()),
    ] {
        let start = Instant::now();
        let (path, v) =
            convert_with_options(&engine, &input, root.path(), format, &options, &off).unwrap();
        assert!(v.icc && v.output_depth == 8);
        if !matches!(format, Format::Jxl) {
            assert!(v.exact);
        }
        assert_eq!(sha256(&mut File::open(&input).unwrap()).unwrap(), before);
        println!(
            "PASS 16 MP {format:?} elapsed={:.1}s {v}",
            start.elapsed().as_secs_f64()
        );
        if matches!(format, Format::Avif)
            && let Some(iat) = std::env::var_os("IMBC_TEST_IAT")
                .map(PathBuf::from)
                .filter(|p| p.is_file())
        {
            let report = root.path().join("large-iat.json");
            let status = Command::new(iat)
                .arg("compare")
                .arg(&input)
                .arg(&path)
                .arg("--report-json")
                .arg(&report)
                .output()
                .unwrap();
            assert_eq!(status.status.code(), Some(0));
            let j: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
            assert_eq!(j["output"]["avif"]["pixel_verified"], true);
            println!("PASS iat 16 MP AVIF pixel_verified=true");
        }
    }
}

#[test]
fn v14_format_and_defaults() {
    assert!(matches!(Format::input(Path::new("a.PSB")), Ok(Format::Psb)));
    assert_eq!(Format::Psb.input_coder(), "psd:-[0]");
    let defaults = Options::default();
    assert!(defaults.jxl_lossless && !defaults.avif_lossless);
}

#[test]
#[ignore = "Requires actual v1.4 runtime; set IMBC_TEST_ENGINE and optionally IMBC_TEST_IAT"]
fn v14_runtime_formats() {
    let Some(engine) = std::env::var_os("IMBC_TEST_ENGINE").map(PathBuf::from) else {
        eprintln!("skipped v14_runtime_formats: set IMBC_TEST_ENGINE");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    let source = dir.path().join("source.png");
    fs::write(
        &source,
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
    )
    .unwrap();
    let before = fs::read(&source).unwrap();
    let avif = Options {
        avif_lossless: true,
        ..Default::default()
    };
    let jxl = Options {
        jxl_lossless: false,
        jxl_quality: 75,
        ..Default::default()
    };
    for (format, options) in [
        (Format::Avif, &avif),
        (Format::Jxl, &jxl),
        (Format::Psd, &Options::default()),
        (Format::Psb, &Options::default()),
    ] {
        let (output, v) = convert_with_options(&engine, &source, dir.path(), format, options, &off)
            .unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert!(v.icc, "{format:?}: {v}");
        assert_eq!(v.output_depth, 8, "{format:?}: {v}");
        if !matches!(format, Format::Jxl) {
            assert!(v.exact, "{format:?}: {v}");
        }
        if matches!(format, Format::Psd | Format::Psb) {
            let bytes = fs::read(&output).unwrap();
            assert_eq!(
                &bytes[..6],
                if matches!(format, Format::Psb) {
                    b"8BPS\0\x02"
                } else {
                    b"8BPS\0\x01"
                }
            );
            let (_, back) = convert_with_options(
                &engine,
                &output,
                dir.path(),
                Format::Png,
                &Options::default(),
                &off,
            )
            .unwrap();
            assert_eq!((back.input_depth, back.output_depth), (8, 8));
            assert!(back.exact && back.icc);
            let color_len = u32::from_be_bytes(bytes[26..30].try_into().unwrap()) as usize;
            let resources = 30 + color_len;
            let layer_section = resources
                + 4
                + u32::from_be_bytes(bytes[resources..resources + 4].try_into().unwrap()) as usize;
            let len_bytes = if matches!(format, Format::Psb) { 8 } else { 4 };
            let count_at = layer_section + len_bytes * 2;
            assert_eq!(
                i16::from_be_bytes(bytes[count_at..count_at + 2].try_into().unwrap())
                    .unsigned_abs(),
                1,
                "single layer required"
            );
            if matches!(format, Format::Psb) {
                let mislabeled = dir.path().join("psb-content.psd");
                fs::write(&mislabeled, &bytes).unwrap();
                let (_, v) = convert_with_options(
                    &engine,
                    &mislabeled,
                    dir.path(),
                    Format::Png,
                    &Options::default(),
                    &off,
                )
                .unwrap();
                assert!(v.exact && v.icc);
            }
        }
        println!("PASS {format:?}: {v}");
        if matches!(format, Format::Avif) {
            if let Some(iat) = std::env::var_os("IMBC_TEST_IAT")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute() && p.is_file())
            {
                let report = dir.path().join("iat.json");
                let result = Command::new(iat)
                    .arg("compare")
                    .arg(&source)
                    .arg(&output)
                    .arg("--report-json")
                    .arg(&report)
                    .output()
                    .unwrap();
                assert_eq!(
                    result.status.code(),
                    Some(0),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let json = fs::read_to_string(report).unwrap();
                assert!(
                    json.contains("\"pixel_verified\": true")
                        || json.contains("\"pixel_verified\":true"),
                    "AVIF not pixel verified"
                );
                println!("PASS iat AVIF output.avif.pixel_verified=true, exit=0");
            } else {
                eprintln!("skipped AVIF external verification: set IMBC_TEST_IAT");
            }
        }
    }
    assert_eq!(fs::read(&source).unwrap(), before);
    let alpha = dir.path().join("alpha.png");
    fs::write(&alpha, include_bytes!("../tests/fixtures/alpha.png")).unwrap();
    let alpha_before = fs::read(&alpha).unwrap();
    for format in [Format::Psd, Format::Psb] {
        let count = fs::read_dir(dir.path()).unwrap().count();
        let e = convert_with_options(
            &engine,
            &alpha,
            dir.path(),
            format,
            &Options::default(),
            &off,
        )
        .unwrap_err();
        assert!(e.contains("Alpha"), "{e}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
    }
    for options in [&avif, &jxl, &Options::default()] {
        let format = if options.avif_lossless {
            Format::Avif
        } else {
            Format::Jxl
        };
        let (encoded, v) =
            convert_with_options(&engine, &alpha, dir.path(), format, options, &off).unwrap();
        if options.avif_lossless || options.jxl_lossless {
            assert!(v.exact);
        }
        println!("PASS Alpha {format:?}: {v}");
        if matches!(format, Format::Avif) {
            let (_, back) = convert_with_options(
                &engine,
                &encoded,
                dir.path(),
                Format::Png,
                &Options::default(),
                &off,
            )
            .unwrap();
            assert!(back.exact && back.color_missing);
        }
    }
    let (tiff, _) = convert_with_options(
        &engine,
        &alpha,
        dir.path(),
        Format::Tiff,
        &Options::default(),
        &off,
    )
    .unwrap();
    let before = fs::read(&tiff).unwrap();
    for format in [Format::Psd, Format::Psb] {
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert!(
            convert_with_options(
                &engine,
                &tiff,
                dir.path(),
                format,
                &Options::default(),
                &off
            )
            .unwrap_err()
            .contains("Alpha")
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
    }
    assert_eq!(fs::read(tiff).unwrap(), before);
    let high = dir.path().join("depth16.png");
    fs::write(&high, include_bytes!("../tests/fixtures/v13-icc16.png")).unwrap();
    for format in [Format::Jxl, Format::Psd, Format::Psb] {
        let options = if matches!(format, Format::Jxl) {
            &jxl
        } else {
            &Options::default()
        };
        let (_, v) =
            convert_with_options(&engine, &high, dir.path(), format, options, &off).unwrap();
        assert_eq!(v.output_depth, 16);
        assert!(v.icc);
        println!("PASS 16-bit {format:?}: {v}");
    }
    let count = fs::read_dir(dir.path()).unwrap().count();
    assert!(
        convert_with_options(&engine, &high, dir.path(), Format::Avif, &avif, &off)
            .unwrap_err()
            .contains("8-bit")
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
    assert_eq!(fs::read(alpha).unwrap(), alpha_before);
}

// 2026-11-18：引擎改在每工作 AppContainer 內執行後，FAT／exFAT／ReFS／遠端來源不再
// 一律拒絕，而是先複製到本機 NTFS 受控暫存，引擎只拿副本的唯讀控制代碼。
// 能力以模擬注入；真實 exFAT 磁碟區的 AppContainer 拒絕存取另以探測程式唯讀驗證。
#[test]
#[ignore = "Runtime preflight with simulated FAT/exFAT capabilities; set IMBC_TEST_ENGINE"]
fn v14_non_acl_filesystem_source_is_isolated_copy_and_unknown_fails_closed() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png");
    fs::write(
        &input,
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
    )
    .unwrap();
    let before = fs::read(&input).unwrap();
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = None);
        }
    }
    let _reset = Reset;
    for (filesystem, flags, remote) in [
        ("FAT32", 0, false),
        ("exFAT", 0, false),
        ("ReFS", 8, false),
        ("NTFS", 8, true),
    ] {
        let out = tempfile::tempdir().unwrap();
        filesystem_safety::TEST_CAPABILITIES
            .with(|v| *v.borrow_mut() = Some((filesystem, flags, remote, true)));
        let copies = TEST_SOURCE_COPIES.with(|v| v.get());
        let (path, verification) = convert_with_options(
            &engine,
            &input,
            out.path(),
            Format::Png,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_or_else(|e| panic!("{filesystem}: {e}"));
        assert_eq!(
            TEST_SOURCE_COPIES.with(|v| v.get()),
            copies + 1,
            "{filesystem}"
        );
        assert!(
            path.is_file() && verification.icc,
            "{filesystem}: {verification}"
        );
        assert_eq!(fs::read(&input).unwrap(), before);
        println!("PASS isolated copy {filesystem} remote={remote}: {verification}");
    }
    // 本機能力無法判定：拒絕且不產生輸出。
    let out = tempfile::tempdir().unwrap();
    filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = Some(("", 0, false, false)));
    let error = convert_with_options(
        &engine,
        &input,
        out.path(),
        Format::Png,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("無法確認"), "{error}");
    assert_eq!(fs::read_dir(out.path()).unwrap().count(), 0);
    // 本機 NTFS 不複製。
    filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = None);
    let copies = TEST_SOURCE_COPIES.with(|v| v.get());
    let out = tempfile::tempdir().unwrap();
    convert_with_options(
        &engine,
        &input,
        out.path(),
        Format::Png,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(TEST_SOURCE_COPIES.with(|v| v.get()), copies);
    assert_eq!(fs::read(&input).unwrap(), before);
}

// ───────────── RC2 第二輪（Codex 獨立複查 H2／H3／M1／M2）─────────────

fn lower(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}
fn full_control(path: &Path) {
    let user = runtime_cache::test_support::current_user();
    runtime_cache::test_support::set_tree_sddl(
        path,
        &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"),
    );
}
fn read_only(path: &Path) {
    let user = runtime_cache::test_support::current_user();
    runtime_cache::test_support::set_tree_sddl(
        path,
        &format!("D:P(A;OICI;0x1200a9;;;{user})(A;OICI;FA;;;SY)"),
    );
}
/// 移除測試快取：檔案與目錄只給使用者刪除權限，`remove_dir_all` 即可。
fn remove_cache(root: &Path) {
    if root.exists() {
        fs::remove_dir_all(root).unwrap_or_else(|e| panic!("{}：{e}", root.display()));
    }
}

/// H2：只複製清單內檔案、逐檔受保護 DACL、鎖定期間不可改寫；唯讀 Portable 不被修改。
#[test]
fn rc2_h2_cache_copies_manifest_files_with_locked_readonly_acl_and_leaves_portable_untouched() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let portable = work.path().join("Portable 快取 (測試) #1 & 中文");
    fs::create_dir_all(portable.join("ImageMagick").join("modules").join("coders")).unwrap();
    let dll = portable.join("ImageMagick").join("a.dll");
    let coder = portable
        .join("ImageMagick")
        .join("modules")
        .join("coders")
        .join("c.dll");
    fs::write(&dll, b"dll bytes").unwrap();
    fs::write(&coder, b"coder bytes").unwrap();
    fs::write(
        portable.join("ImageMagick").join("private.png"),
        b"user image",
    )
    .unwrap();
    read_only(&portable);
    let before = (sddl_of(&portable.join("ImageMagick")), sddl_of(&dll));
    let root = work.path().join("cache 根 目錄");
    let files = [
        ("ImageMagick/a.dll", dll.as_path(), None),
        ("ImageMagick/modules/coders/c.dll", coder.as_path(), None),
    ];
    let (key_dir, pins) = prepare_at(&root, "1.0.0-test", &files).unwrap();
    let cached = key_dir.join("ImageMagick").join("a.dll");
    assert_eq!(fs::read(&cached).unwrap(), b"dll bytes");
    assert_eq!(
        fs::read(
            key_dir
                .join("ImageMagick")
                .join("modules")
                .join("coders")
                .join("c.dll")
        )
        .unwrap(),
        b"coder bytes"
    );
    // 清單外的檔案（例如使用者放在 runtime 樹內的圖片）不會進入快取，也不會被授權。
    assert!(!key_dir.join("ImageMagick").join("private.png").exists());
    // Portable 的 DACL 完全未變。
    assert_eq!(
        (sddl_of(&portable.join("ImageMagick")), sddl_of(&dll)),
        before
    );
    // 快取物件：受保護、無繼承 ACE、只授權使用者／SYSTEM／runtime capability。
    let user = current_user();
    for path in [&cached, &key_dir.join("ImageMagick"), &key_dir] {
        let text = sddl_of(path);
        assert!(text.starts_with("D:P"), "{text}");
        assert!(
            !text.contains("ID;") && !text.contains("OI") && !text.contains("CI"),
            "{text}"
        );
        assert!(
            text.contains(low_integrity::RUNTIME_CAPABILITY) && text.contains(&user),
            "{text}"
        );
        assert!(
            !text.contains("S-1-15-2-1)") && !text.contains(";AC)"),
            "{text}"
        );
    }
    // 鎖定期間：無法寫入、刪除或改名。
    assert!(fs::OpenOptions::new().append(true).open(&cached).is_err());
    assert!(fs::remove_file(&cached).is_err());
    assert!(fs::rename(key_dir.join("ImageMagick"), key_dir.join("moved")).is_err());
    assert!(fs::rename(&key_dir, root.join("moved")).is_err());
    drop(pins);
    // 解除鎖定後，DACL 本身仍拒絕使用者寫入與新增檔案。
    let denied = fs::OpenOptions::new()
        .append(true)
        .open(&cached)
        .unwrap_err();
    assert_eq!(denied.raw_os_error(), Some(5), "{denied}");
    let planted = fs::write(key_dir.join("ImageMagick").join("planted.dll"), b"x").unwrap_err();
    assert_eq!(planted.raw_os_error(), Some(5), "{planted}");
    // 再次使用：驗證既有快取即可，不需要讀取 Portable（讓來源完全不可讀以證明）。
    let hashes = (sha(&dll).unwrap(), sha(&coder).unwrap());
    runtime_cache::test_support::set_tree_sddl(&portable, "D:P(A;OICI;FA;;;SY)");
    let again = [
        ("ImageMagick/a.dll", dll.as_path(), Some(hashes.0)),
        (
            "ImageMagick/modules/coders/c.dll",
            coder.as_path(),
            Some(hashes.1),
        ),
    ];
    let (same, pins) = prepare_at(&root, "1.0.0-test", &again).unwrap();
    assert_eq!(lower(&same), lower(&key_dir));
    drop(pins);
    full_control(&portable);
    remove_cache(&root);
}

/// H2：清單雜湊不符拒絕且不留半成品；被竄改或被植入檔案的快取重建，
/// 竄改／植入的內容不刪除，完整保留在 quarantine-*（第三輪預設拒絕刪除）。
#[test]
fn rc2_h2_cache_rejects_mismatch_and_rebuilds_tampered_or_planted_cache() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("magick.exe");
    fs::write(&source, b"engine bytes").unwrap();
    let root = work.path().join("cache");
    let error = prepare_at(
        &root,
        "bad",
        &[("ImageMagick/magick.exe", &source, Some([7; 32]))],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("SHA-256 不符"), "{error}");
    // 只剩所有權登錄檔；staging 與版本目錄都不留下。
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["owned.imbc"], "不得留下 staging 或版本目錄");
    let files = [("ImageMagick/magick.exe", source.as_path(), None)];
    let (key_dir, pins) = prepare_at(&root, "good", &files).unwrap();
    drop(pins);
    let cached = key_dir.join("ImageMagick").join("magick.exe");
    // 擁有者仍可改 DACL：模擬同使用者程式改寫內容。
    full_control(&cached);
    fs::write(&cached, b"tampered").unwrap();
    let (rebuilt, pins) = prepare_at(&root, "good", &files).unwrap();
    assert_eq!(lower(&rebuilt), lower(&key_dir));
    assert_eq!(fs::read(&cached).unwrap(), b"engine bytes");
    assert!(sddl_of(&cached).starts_with("D:P"));
    drop(pins);
    // 植入清單外的 DLL：目錄內容與清單不符，整個版本目錄重建。
    full_control(&key_dir.join("ImageMagick"));
    fs::write(
        key_dir.join("ImageMagick").join("IM_MOD_RL_evil_.dll"),
        b"x",
    )
    .unwrap();
    let (_, pins) = prepare_at(&root, "good", &files).unwrap();
    assert!(
        !key_dir
            .join("ImageMagick")
            .join("IM_MOD_RL_evil_.dll")
            .exists()
    );
    drop(pins);
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let quarantined: Vec<_> = names
        .iter()
        .filter(|n| n.starts_with("quarantine-"))
        .collect();
    assert_eq!(names.len(), 4, "{names:?}");
    assert!(names.contains(&"good".to_owned()) && names.contains(&"owned.imbc".to_owned()));
    assert_eq!(quarantined.len(), 2, "{names:?}");
    // 被竄改的檔案與植入的 DLL 都原樣保留，沒有被刪除。
    let kept: Vec<Vec<u8>> = quarantined
        .iter()
        .flat_map(|q| fs::read_dir(root.join(q).join("ImageMagick")).unwrap())
        .map(|e| fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        kept.contains(&b"tampered".to_vec()) && kept.contains(&b"x".to_vec()),
        "{kept:?}"
    );
    remove_cache(&root);
}

/// H2：版本更新改用新目錄；舊版快取在未使用時清除，使用中（被鎖定）與進行中的建立保留。
#[test]
fn rc2_h2_version_update_uses_new_cache_and_cleanup_keeps_in_use_entries() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let old = work.path().join("old.exe");
    let new = work.path().join("new.exe");
    fs::write(&old, b"version 1").unwrap();
    fs::write(&new, b"version 2").unwrap();
    let root = work.path().join("cache");
    let (in_use, held) = prepare_at(&root, "1.0.0-aaaa", &[("JXL/cjxl.exe", &old, None)]).unwrap();
    let (stale, pins) = prepare_at(&root, "1.0.1-bbbb", &[("JXL/cjxl.exe", &old, None)]).unwrap();
    drop(pins);
    fs::create_dir(root.join("staging-inprogress")).unwrap();
    reset_cleanup(&root);
    let (current, pins) = prepare_at(&root, "1.1.0-cccc", &[("JXL/cjxl.exe", &new, None)]).unwrap();
    assert_eq!(
        fs::read(current.join("JXL").join("cjxl.exe")).unwrap(),
        b"version 2"
    );
    assert!(!stale.exists(), "未使用的舊版快取應移除");
    assert_eq!(
        fs::read(in_use.join("JXL").join("cjxl.exe")).unwrap(),
        b"version 1"
    );
    assert!(
        root.join("staging-inprogress").exists(),
        "10 分鐘內的建立中目錄應保留"
    );
    assert!(fs::read_dir(&root).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("trash-")
    }));
    drop((held, pins));
    remove_cache(&root);
}

/// H2：多個執行個體同時建立同一版本快取時收斂到同一份已驗證快取，不留 staging。
#[test]
fn rc2_h2_concurrent_builders_converge_on_one_verified_cache() {
    use runtime_cache::test_support::*;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("cjxl.exe");
    fs::write(&source, vec![5u8; 4 << 20]).unwrap();
    let root = work.path().join("cache");
    let prepared: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    prepare_at(&root, "1.0.0-race", &[("JXL/cjxl.exe", &source, None)]).unwrap()
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    assert!(
        prepared
            .iter()
            .all(|(dir, _)| lower(dir) == lower(&prepared[0].0))
    );
    drop(prepared);
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["1.0.0-race", "owned.imbc"], "{names:?}");
    remove_cache(&root);
}

/// H2：AppContainer 引擎可讀取快取，但無法改寫快取檔案或在快取目錄植入檔案。
#[cfg(windows)]
#[test]
fn rc2_h2_appcontainer_reads_cache_but_cannot_modify_or_plant() {
    use super::regression_security::{ENGINE_FIXTURE, fixture_exe, fixture_input};
    let job = Job::new().unwrap();
    let exe = fixture_exe();
    let cached = runtime_cache::resolve(&exe).unwrap();
    assert_ne!(lower(&cached), lower(&exe), "引擎必須從快取啟動");
    let expected = runtime_cache::test_support::sha(&exe).unwrap();
    let script = format!(
        "readfile {0}\nwritefile {0}\ncreatein {1}\nstdin\n",
        cached.display(),
        cached.parent().unwrap().display()
    );
    let input = fixture_input(&script);
    let mut output = job_file().unwrap();
    run(
        &exe,
        &ENGINE_FIXTURE,
        &input,
        &output,
        &AtomicBool::new(false),
    )
    .unwrap();
    output.seek(SeekFrom::Start(0)).unwrap();
    let mut text = String::new();
    output.read_to_string(&mut text).unwrap();
    for marker in ["READ_OK=true", "WRITE_DENIED=true", "CREATE_DENIED=true"] {
        assert!(text.contains(marker), "{marker}: {text}");
    }
    assert!(!cached.parent().unwrap().join("planted.dll").exists());
    assert_eq!(runtime_cache::test_support::sha(&cached).unwrap(), expected);
    drop((input, output));
    job.finish(Ok(())).unwrap();
}

struct ResetCapabilities;
impl Drop for ResetCapabilities {
    fn drop(&mut self) {
        filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = None);
    }
}

/// M1：複製來源與交付輸出期間取消，回傳精確的「已取消」，且不在目的端留下檔案。
#[test]
fn rc2_m1_cancellation_during_copy_and_delivery_stays_exact() {
    use std::io::Write;
    let job = Job::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.png");
    fs::write(&source, vec![1u8; 1 << 20]).unwrap();
    {
        let _reset = ResetCapabilities;
        filesystem_safety::TEST_CAPABILITIES
            .with(|v| *v.borrow_mut() = Some(("exFAT", 0, false, true)));
        let error = isolated_source(File::open(&source).unwrap(), &AtomicBool::new(true))
            .err()
            .unwrap();
        assert_eq!(error, "已取消");
    }
    let out = tempfile::tempdir().unwrap();
    let destination = Destination::open(out.path()).unwrap();
    let mut local = job_file().unwrap();
    local.write_all(&[9u8; 1 << 20]).unwrap();
    let error = deliver(
        &mut local,
        &destination,
        std::ffi::OsStr::new("source"),
        Format::Png,
        &AtomicBool::new(true),
    )
    .unwrap_err();
    assert_eq!(error, "已取消");
    assert_eq!(
        fs::read_dir(out.path()).unwrap().count(),
        0,
        "取消不得留下暫存或輸出"
    );
    drop(local);
    // 有一般提示（例如暫存根目錄改用本機）時，取消仍必須是精確的「已取消」。
    WARNINGS.with(|w| {
        w.borrow_mut()
            .push("暫存根目錄改用本機受控暫存（測試提示）".into())
    });
    assert_eq!(
        job.finish_with_warnings::<()>(Err("已取消".into()))
            .unwrap_err(),
        "已取消"
    );
}

/// M2：目的目錄固定期間不可改名；交付以同一控制代碼建立、讀回比對並不覆寫發布。
#[test]
fn rc2_m2_destination_handle_pins_directory_and_never_overwrites() {
    use std::io::Write;
    let job = Job::new().unwrap();
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("輸出 目錄 #2 (測試)");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("image.png"), b"original").unwrap();
    let destination = Destination::open(&target).unwrap();
    assert!(fs::rename(&target, out.path().join("moved")).is_err());
    let mut local = job_file().unwrap();
    local.write_all(b"validated bytes").unwrap();
    let published = deliver(
        &mut local,
        &destination,
        std::ffi::OsStr::new("image"),
        Format::Png,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(published.file_name().unwrap(), "image (1).png");
    assert_eq!(fs::read(&published).unwrap(), b"validated bytes");
    assert_eq!(fs::read(target.join("image.png")).unwrap(), b"original");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 2, "不得留下 .tmp");
    // 未發布的暫存釋放時以控制代碼刪除。
    let temp = destination.create_temp().unwrap();
    let leftover = temp.path().to_owned();
    assert!(leftover.exists() && leftover.parent() == Some(destination.path()));
    drop(temp);
    assert!(!leftover.exists());
    drop((local, destination));
    fs::rename(&target, out.path().join("moved")).unwrap();
    job.finish(Ok(())).unwrap();
}

/// H3：交給引擎的 stdin 控制代碼只能是受控暫存（或本機 NTFS 原始來源），
/// 絕不是目的端檔案；非本機 NTFS 來源也不會直接交出。一般格式與 JPEG ↔ JXL 皆驗證。
#[test]
#[ignore = "Runtime handle-boundary regression; set IMBC_TEST_ENGINE"]
fn rc2_h3_engine_inputs_are_controlled_copies_never_destination() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let tools = engine.parent().unwrap().parent().unwrap().join("JXL");
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png");
    fs::write(
        &input,
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
    )
    .unwrap();
    let jpeg = dir.path().join("photo.jpg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jxl-compat/s444-q95.jpg"),
        &jpeg,
    )
    .unwrap();
    let before = (fs::read(&input).unwrap(), fs::read(&jpeg).unwrap());
    let temp_root = lower(
        &controlled_temp::preferred_root()
            .unwrap()
            .canonicalize()
            .unwrap(),
    );
    let _reset = ResetCapabilities;
    for caps in [
        Some(("exFAT", 0, false, true)),
        Some(("FAT32", 0, false, true)),
        Some(("NTFS", 8, true, true)),
        None,
    ] {
        filesystem_safety::TEST_CAPABILITIES.with(|v| *v.borrow_mut() = caps);
        for (label, source) in [("png", &input), ("jxl-wrap", &jpeg)] {
            let out = tempfile::tempdir().unwrap();
            TEST_ENGINE_INPUTS.with(|v| v.borrow_mut().clear());
            let published = if label == "png" {
                let (path, verification) = convert_with_options(
                    &engine,
                    source,
                    out.path(),
                    Format::Png,
                    &Options::default(),
                    &AtomicBool::new(false),
                )
                .unwrap_or_else(|e| panic!("{caps:?} {label}: {e}"));
                assert!(verification.exact && verification.icc, "{verification}");
                path
            } else {
                let (path, _, verified) = jpeg_reconstruction(
                    &tools,
                    source,
                    out.path(),
                    true,
                    None,
                    &AtomicBool::new(false),
                )
                .unwrap_or_else(|e| panic!("{caps:?} {label}: {e}"));
                assert!(verified);
                path
            };
            assert!(published.is_file());
            let inputs = TEST_ENGINE_INPUTS.with(|v| std::mem::take(&mut *v.borrow_mut()));
            assert!(!inputs.is_empty());
            let destination = lower(&out.path().canonicalize().unwrap());
            for path in &inputs {
                let path = lower(path);
                assert!(!path.starts_with("err "), "{caps:?} {label}: {path}");
                assert!(
                    !path.starts_with(&destination),
                    "{caps:?} {label}: 引擎取得目的端控制代碼 {path}"
                );
                if caps.is_some() || label == "jxl-wrap" {
                    // 非本機 NTFS 來源與 JPEG ↔ JXL：一律只交出受控暫存副本。
                    assert!(path.starts_with(&temp_root), "{caps:?} {label}: {path}");
                } else {
                    assert!(
                        path.starts_with(&temp_root)
                            || path == lower(&source.canonicalize().unwrap()),
                        "{label}: {path}"
                    );
                }
            }
            println!(
                "PASS {caps:?} {label}: {} 次引擎呼叫皆為受控控制代碼",
                inputs.len()
            );
        }
    }
    assert_eq!(
        (fs::read(&input).unwrap(), fs::read(&jpeg).unwrap()),
        before
    );
}

/// H3／M2 實體媒體：真實 exFAT（卸除式）與 SMB 上的來源與輸出。只在指定根目錄下建立本測試
/// 專用的暫存資料夾（結束時刪除），不讀寫其他檔案。不使用能力注入。
#[test]
#[ignore = "Real exFAT/SMB media; set IMBC_TEST_ENGINE and IMBC_TEST_EXFAT_ROOT and/or IMBC_TEST_UNC_ROOT"]
fn rc2_h3_real_exfat_and_smb_sources_and_outputs_use_controlled_handles() {
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let tools = engine.parent().unwrap().parent().unwrap().join("JXL");
    let temp_root = lower(
        &controlled_temp::preferred_root()
            .unwrap()
            .canonicalize()
            .unwrap(),
    );
    let mut roots = Vec::new();
    for (label, var) in [
        ("exFAT", "IMBC_TEST_EXFAT_ROOT"),
        ("SMB", "IMBC_TEST_UNC_ROOT"),
    ] {
        if let Some(root) = std::env::var_os(var) {
            roots.push((label, PathBuf::from(root)));
        }
    }
    assert!(!roots.is_empty());
    for (label, root) in roots {
        let media = tempfile::Builder::new()
            .prefix("imb-c-rc2-media-")
            .tempdir_in(&root)
            .unwrap();
        let source = media.path().join("來源 影像.png");
        fs::write(
            &source,
            include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
        )
        .unwrap();
        let jpeg = media.path().join("photo.jpg");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jxl-compat/s444-q95.jpg"),
            &jpeg,
        )
        .unwrap();
        let before = (
            sha256(&mut File::open(&source).unwrap()).unwrap(),
            sha256(&mut File::open(&jpeg).unwrap()).unwrap(),
        );
        // 實際判定：來源必須走複製，輸出可用。
        let protection = filesystem_safety::check(&File::open(&source).unwrap()).unwrap();
        assert!(
            matches!(&protection, filesystem_safety::Protection::CopyFirst(_)),
            "{label}: {protection:?}"
        );
        let out = media.path().join("輸出");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("來源 影像.png"), b"existing").unwrap();
        {
            let pinned = Destination::open(&out).unwrap();
            assert!(
                fs::rename(&out, media.path().join("moved")).is_err(),
                "{label}: 固定期間目的目錄不得可改名"
            );
            drop(pinned);
        }
        TEST_ENGINE_INPUTS.with(|v| v.borrow_mut().clear());
        let copies = TEST_SOURCE_COPIES.with(|v| v.get());
        let (png, verification) = convert_with_options(
            &engine,
            &source,
            &out,
            Format::Png,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_or_else(|e| panic!("{label} png: {e}"));
        assert!(
            verification.exact && verification.icc,
            "{label}: {verification}"
        );
        assert_eq!(TEST_SOURCE_COPIES.with(|v| v.get()), copies + 1, "{label}");
        assert_eq!(
            png.file_name().unwrap(),
            "來源 影像 (1).png",
            "{label}: 同名不得覆寫"
        );
        assert_eq!(fs::read(out.join("來源 影像.png")).unwrap(), b"existing");
        let (jxl, _, verified) =
            jpeg_reconstruction(&tools, &jpeg, &out, true, None, &AtomicBool::new(false))
                .unwrap_or_else(|e| panic!("{label} jxl: {e}"));
        assert!(verified && jxl.is_file());
        let media_lower = lower(&media.path().canonicalize().unwrap());
        let inputs = TEST_ENGINE_INPUTS.with(|v| std::mem::take(&mut *v.borrow_mut()));
        for path in &inputs {
            let path = lower(path);
            assert!(
                path.starts_with(&temp_root),
                "{label}: 引擎取得非受控控制代碼 {path}"
            );
            assert!(!path.contains(&media_lower), "{label}: {path}");
        }
        assert_eq!(
            (
                sha256(&mut File::open(&source).unwrap()).unwrap(),
                sha256(&mut File::open(&jpeg).unwrap()).unwrap()
            ),
            before,
            "{label}: 來源 SHA-256 改變"
        );
        let leftovers: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{label}: {leftovers:?}");
        println!(
            "PASS real {label}: {protection:?}; {} 次引擎呼叫皆為本機受控暫存；同名保留；來源 SHA-256 不變",
            inputs.len()
        );
    }
}

/// H2：唯讀 Portable 目錄（使用者只有讀取／執行）可正常轉檔，且 ACL 與內容都不被修改。
#[test]
#[ignore = "Copies the stage runtime into a read-only portable folder; set IMBC_TEST_ENGINE"]
fn rc2_h2_readonly_portable_runtime_converts_and_stays_unmodified() {
    use runtime_cache::test_support::*;
    let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
    let stage = engine.parent().unwrap().parent().unwrap().to_owned();
    let work = tempfile::tempdir().unwrap();
    let portable = work.path().join("唯讀 Portable (RC2) & 測試 #3");
    for rel in production_rels() {
        let target = portable.join(rel.replace('/', "\\"));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(stage.join(rel.replace('/', "\\")), &target).unwrap();
    }
    let private = portable.join("ImageMagick").join("私人圖片.png");
    fs::write(&private, b"not part of runtime").unwrap();
    read_only(&portable);
    let snapshot = |root: &Path| {
        let mut items = Vec::new();
        for entry in walk(root) {
            items.push((lower(&entry), sddl_of(&entry)));
        }
        items
    };
    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = vec![root.to_owned()];
        if root.is_dir() {
            let mut children: Vec<_> = fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                out.extend(walk(&child));
            }
        }
        out
    }
    let before = snapshot(&portable);
    let cache_root = work.path().join("cache");
    let key_dir = prepare_production(&portable, &cache_root).unwrap();
    let portable_engine = portable.join("ImageMagick").join("magick.exe");
    let input = work.path().join("source.png");
    fs::write(
        &input,
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    TEST_ENGINE_INPUTS.with(|v| v.borrow_mut().clear());
    let (path, verification) = convert_with_options(
        &portable_engine,
        &input,
        out.path(),
        Format::Jxl,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(path.is_file() && verification.exact, "{verification}");
    let jpeg = work.path().join("photo.jpg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jxl-compat/s444-q95.jpg"),
        &jpeg,
    )
    .unwrap();
    let (_, _, verified) = jpeg_reconstruction(
        &portable.join("JXL"),
        &jpeg,
        out.path(),
        true,
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(verified);
    assert_eq!(snapshot(&portable), before, "Portable 的檔案或 ACL 被修改");
    assert!(!key_dir.join("ImageMagick").join("私人圖片.png").exists());
    println!("PASS read-only portable: cache {}", key_dir.display());
    unregister(&key_dir);
    full_control(&portable);
    remove_cache(&cache_root);
}
