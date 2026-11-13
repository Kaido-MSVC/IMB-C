//! Targeted regressions from REVIEW-2026-11-10.md; fixtures are never overwritten.
use super::*;
use std::{fs, io::Write};

fn engine() -> PathBuf {
    PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").expect("set IMBC_TEST_ENGINE"))
}
fn icc_png(dir: &Path) -> PathBuf {
    let src = dir.join("icc8.png");
    fs::write(
        &src,
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
    )
    .unwrap();
    src
}
fn resource(id: u16, signature: &[u8; 4], data: &[u8], pad: bool) -> Vec<u8> {
    let mut bytes = signature.to_vec();
    bytes.extend(id.to_be_bytes());
    bytes.extend([0, 0]);
    bytes.extend((data.len() as u32).to_be_bytes());
    bytes.extend(data);
    if pad && data.len() % 2 != 0 {
        bytes.push(0);
    }
    bytes
}
fn psd_resources(bytes: &[u8]) -> (usize, usize) {
    let at = 30 + u32::from_be_bytes(bytes[26..30].try_into().unwrap()) as usize;
    (
        at,
        at + 4 + u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize,
    )
}
fn append_resources(base: &[u8], suffix: &[u8]) -> Vec<u8> {
    let (at, end) = psd_resources(base);
    [
        base[..at].to_vec(),
        ((end - at - 4 + suffix.len()) as u32)
            .to_be_bytes()
            .to_vec(),
        base[at + 4..end].to_vec(),
        suffix.to_vec(),
        base[end..].to_vec(),
    ]
    .concat()
}

#[test]
#[ignore = "v1.3 runtime"]
fn r2_icc_all_format_sources_outputs_and_webp() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let src = icc_png(dir.path());
    let off = AtomicBool::new(false);
    let formats = [
        Format::Png,
        Format::Jpeg,
        Format::Webp,
        Format::Avif,
        Format::Psd,
        Format::Tiff,
        Format::Jxl,
    ];
    let mut webp = None;
    for format in formats {
        let (path, v) =
            convert_with_options(&e, &src, dir.path(), format, &Options::default(), &off)
                .unwrap_or_else(|err| panic!("ICC PNG -> {format:?}: {err}"));
        assert!(v.icc && v.color_verified, "{format:?}: {v}");
        let (_, back) = convert_with_options(
            &e,
            &path,
            dir.path(),
            Format::Png,
            &Options::default(),
            &off,
        )
        .unwrap_or_else(|err| panic!("ICC {format:?} -> PNG: {err}"));
        assert!(back.icc && back.color_verified, "{format:?}: {back}");
        if matches!(format, Format::Webp) {
            webp = Some(path);
        }
    }
    let webp = webp.unwrap();
    for format in [Format::Tiff, Format::Webp, Format::Jxl] {
        let (_, v) =
            convert_with_options(&e, &webp, dir.path(), format, &Options::default(), &off).unwrap();
        assert!(
            v.icc && v.color_verified && v.exact,
            "WebP -> {format:?}: {v}"
        );
    }
    let options = Options {
        target_icc: Some(PathBuf::from(
            r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm",
        )),
        ..Default::default()
    };
    let (_, v) = convert_with_options(&e, &webp, dir.path(), Format::Png, &options, &off).unwrap();
    assert!(v.icc && v.color_verified && v.transformed && v.exact);
    assert_eq!(
        fs::read(&src).unwrap(),
        include_bytes!("../tests/fixtures/acceptance-v13/icc8.png")
    );
    println!(
        "PASS 18 ICC conversion cases; 7 format sources/outputs, WebP all requested targets and target ICC"
    );
}

#[test]
#[ignore = "v1.3 runtime"]
fn r2_psd_engine_tolerance_and_bounded_normalization() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    let src = icc_png(dir.path());
    let (base, _) =
        convert_with_options(&e, &src, dir.path(), Format::Psd, &Options::default(), &off).unwrap();
    let base = fs::read(base).unwrap();
    let variants = [
        ("MeSa", resource(4001, b"MeSa", b"ab", true)),
        ("AgHg", resource(4001, b"AgHg", b"ab", true)),
        ("PHUT", resource(4001, b"PHUT", b"ab", true)),
        ("DCSR", resource(4001, b"DCSR", b"ab", true)),
        ("residual", vec![0; 7]),
        ("odd", resource(4001, b"8BIM", b"abc", false)),
    ];
    for (name, suffix) in variants {
        let bytes = append_resources(&base, &suffix);
        let input = dir.path().join(format!("{name}.psd"));
        fs::write(&input, &bytes).unwrap();
        // Baseline direct decode, without IMB-C normalization.
        let original = File::open(&input).unwrap();
        let raw = tempfile::tempfile().unwrap();
        run(&e, &["psd:-[0]", "png:-"], &original, &raw, &off)
            .unwrap_or_else(|err| panic!("baseline {name}: {err}"));
        assert!(raw.metadata().unwrap().len() > 0);
        let (_, v) = convert_with_options(
            &e,
            &input,
            dir.path(),
            Format::Png,
            &Options::default(),
            &off,
        )
        .unwrap_or_else(|err| panic!("compatible {name}: {err}"));
        assert!(v.icc && v.exact);
        assert_eq!(fs::read(&input).unwrap(), bytes);
        // Empty name resource preceding the tolerant suffix must still be removed,
        // and the unrecognized bytes and image data remain exactly unchanged.
        let empty = resource(1006, b"8BIM", b"", true);
        let mixed = append_resources(&base, &[empty, suffix].concat());
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&mixed).unwrap();
        let mut normalized = psd_compatible_source(&mut file, &off).unwrap().unwrap();
        let mut got = Vec::new();
        normalized.read_to_end(&mut got).unwrap();
        assert_eq!(got, bytes);
        fs::write(&input, &mixed).unwrap();
        let (_, v) = convert_with_options(
            &e,
            &input,
            dir.path(),
            Format::Png,
            &Options::default(),
            &off,
        )
        .unwrap();
        assert!(v.exact && v.icc);
    }
    let mut bad = resource(4001, b"8BIM", b"x", true);
    bad[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    for bytes in [append_resources(&base, &bad), {
        let mut b = base.clone();
        b[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        b
    }] {
        let input = dir.path().join("malicious.psd");
        fs::write(&input, &bytes).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        let err = convert_with_options(
            &e,
            &input,
            dir.path(),
            Format::Png,
            &Options::default(),
            &off,
        )
        .unwrap_err();
        assert!(err.contains("範圍") || err.contains("溢位"), "{err}");
        assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
    }
}

#[test]
#[ignore = "v1.3 runtime"]
fn r2_avif_legacy_untagged_and_hdr_guards() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    let input = dir.path().join("legacy.avif");
    let source = tempfile::tempfile().unwrap();
    (&source)
        .write_all(include_bytes!("../tests/fixtures/v13-hidden.png"))
        .unwrap();
    (&source).seek(SeekFrom::Start(0)).unwrap();
    let output = File::create(&input).unwrap();
    // Exactly the former v1.3 unprofiled AVIF encoding: no nclx define.
    run(
        &e,
        &[
            "png:-",
            "-quality",
            "90",
            "-define",
            "heic:chroma=444",
            "avif:-",
        ],
        &source,
        &output,
        &off,
    )
    .unwrap();
    drop(output);
    let before = fs::read(&input).unwrap();
    for format in [
        Format::Png,
        Format::Tiff,
        Format::Webp,
        Format::Jxl,
        Format::Avif,
    ] {
        let (_, v) =
            convert_with_options(&e, &input, dir.path(), format, &Options::default(), &off)
                .unwrap();
        assert!(!v.icc && !v.color_verified && !v.warnings.is_empty(), "{v}");
        assert!(v.to_string().contains("not verified"));
    }
    let options = Options {
        target_icc: Some(PathBuf::from(
            r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm",
        )),
        ..Default::default()
    };
    let count = fs::read_dir(dir.path()).unwrap().count();
    assert!(
        convert_with_options(&e, &input, dir.path(), Format::Png, &options, &off)
            .unwrap_err()
            .contains("沒有 ICC")
    );
    assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
    assert_eq!(fs::read(&input).unwrap(), before);
    let hdr = dir.path().join("hdr.avif");
    fs::write(&hdr, include_bytes!("../tests/fixtures/v13-cicp-pq.avif")).unwrap();
    assert!(
        convert_with_options(&e, &hdr, dir.path(), Format::Png, &Options::default(), &off)
            .unwrap_err()
            .contains("CICP")
    );
}

#[test]
fn r2_policy_listing_rejects_external_active_rules() {
    let dir = tempfile::tempdir().unwrap();
    let private = dir.path().join("policy.xml");
    fs::write(&private, b"test").unwrap();
    let neutral = format!(
        "Path: {}\n Policy: Coder\n rights: None\nPath: [built-in]\n Policy: Undefined\n rights: None\n",
        private.display()
    );
    validate_policy_listing(&neutral, &private).unwrap();
    for domain in [
        "Coder", "Module", "Delegate", "Filter", "Path", "Resource", "System",
    ] {
        let bad =
            format!("{neutral}Path: [built-in]\n Policy: {domain}\n rights: Read Write Execute\n");
        assert!(validate_policy_listing(&bad, &private).is_err());
    }
    assert!(validate_policy_listing("Path: [built-in]\n Policy: Undefined", &private).is_err());
}

#[test]
#[ignore = "v1.3 runtime"]
fn r2_policy_actual_broad_grant_detected_without_system_changes() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("policy.xml");
    fs::write(
        &policy,
        b"<policymap><policy domain='coder' rights='read|write' pattern='*'/></policymap>",
    )
    .unwrap();
    let mut cmd = super::tests::inspect(&e);
    let paths = std::env::join_paths([e.parent().unwrap(), dir.path()]).unwrap();
    let result = cmd
        .env("MAGICK_CONFIGURE_PATH", paths)
        .args(["-list", "policy"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let listing = String::from_utf8_lossy(&result.stdout);
    validate_policy_listing(&listing, &e.parent().unwrap().join("policy.xml")).unwrap();
    assert!(!listing.contains(&dir.path().display().to_string()));
    verify_active_policy(&e, &AtomicBool::new(false)).unwrap();
}

#[cfg(windows)]
#[test]
fn r2_cleanup_failure_keeps_published_success_and_warning() {
    use std::os::windows::fs::OpenOptionsExt;
    let dest = tempfile::tempdir().unwrap();
    let sentinel = dest.path().join("keep.png");
    fs::write(&sentinel, b"original").unwrap();
    let job = Job::new().unwrap();
    let root = job_dir();
    let hold = File::options()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(root.join("held-by-fixture"))
        .unwrap();
    let mut output = NamedTempFile::new_in(dest.path()).unwrap();
    output.write_all(b"complete validated output").unwrap();
    let published = publish(
        output,
        dest.path(),
        std::ffi::OsStr::new("keep"),
        Format::Png,
    )
    .unwrap();
    let (path, warnings) = job.finish_with_warnings(Ok(published)).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"complete validated output");
    assert_eq!(fs::read(sentinel).unwrap(), b"original");
    assert!(!warnings.is_empty() && warnings[0].contains(&root.display().to_string()));
    drop(hold);
    assert!(root.starts_with(controlled_temp::preferred_root().unwrap()));
    fs::remove_dir_all(root).unwrap();
}
#[cfg(windows)]
#[test]
fn r2_cleanup_failure_does_not_mask_verification_failure() {
    use std::os::windows::fs::OpenOptionsExt;
    let job = Job::new().unwrap();
    let root = job_dir();
    let hold = File::options()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(root.join("held-by-fixture"))
        .unwrap();
    let error = job
        .finish_with_warnings::<()>(Err("pixel verification failed".into()))
        .unwrap_err();
    assert!(error.contains("pixel verification failed") && error.contains("清理失敗"));
    drop(hold);
    assert!(root.starts_with(controlled_temp::preferred_root().unwrap()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Subprocess fixture only; run r2_anonymous_temp_survives_clone_then_os_cleans_after_exit"]
fn r2_child_temp_fixture() {
    let Some(root) = std::env::var_os("IMBC_R2_CHILD_TEMP_ROOT") else {
        return;
    };
    let _job = Job::new_in(Path::new(&root)).unwrap();
    let mut file = job_file().unwrap();
    file.write_all(b"decoded private pixels").unwrap();
    let mut clone = file.try_clone().unwrap();
    drop(file);
    clone.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    clone.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"decoded private pixels");
    std::process::exit(0); // Bypass destructors deliberately; OS must close/delete the anonymous file.
}
#[test]
fn r2_anonymous_temp_survives_clone_then_os_cleans_after_exit() {
    let dir = tempfile::tempdir().unwrap();
    let peer = dir.path().join("external-sentinel");
    fs::write(&peer, b"keep").unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "engine::regression_r2::r2_child_temp_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("IMBC_R2_CHILD_TEMP_ROOT", dir.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let jobs: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        fs::read_dir(&jobs[0]).unwrap().count(),
        0,
        "anonymous file survived forced destructor bypass"
    );
    assert_eq!(fs::read(peer).unwrap(), b"keep");
}

#[test]
#[ignore = "v1.3 runtime"]
fn r2_temp_long_special_paths_and_exclusive_names() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let mut root = dir.path().join("中文 %03d [1] & %TEMP%");
    while root.as_os_str().len() < 290 {
        root = root.join("long-path-component-0123456789");
    }
    fs::create_dir_all(&root).unwrap();
    let src = icc_png(&root);
    let off = AtomicBool::new(false);
    let job = Job::new_in(&root).unwrap();
    let job_root = job_dir();
    let f = job_file().unwrap();
    assert!(f.metadata().unwrap().is_file());
    drop(f);
    let mut names = std::collections::HashSet::new();
    let mut files = Vec::new();
    for _ in 0..32 {
        let f = tempfile::Builder::new()
            .rand_bytes(16)
            .prefix("icc-")
            .suffix(".icc")
            .tempfile_in(&job_root)
            .unwrap();
        assert!(names.insert(f.path().to_owned()));
        files.push(f);
    }
    drop(files);
    let options = Options {
        target_icc: Some(PathBuf::from(
            r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm",
        )),
        ..Default::default()
    };
    let result = convert_with_options_inner(&e, &src, &root, Format::Png, &options, &off);
    let ((_, v), warnings) = job.finish_with_warnings(result).unwrap();
    assert!(
        v.icc && v.color_verified && v.exact && warnings.is_empty(),
        "{v:?}; warnings={warnings:?}"
    );
    assert!(!job_root.exists());
}

#[cfg(windows)]
#[test]
#[ignore = "v1.3 runtime and accessible localhost administrative share; set IMBC_TEST_UNC_ROOT"]
fn r2_temp_unc_paths() {
    let e = engine();
    let root = PathBuf::from(
        std::env::var_os("IMBC_TEST_UNC_ROOT")
            .expect("set IMBC_TEST_UNC_ROOT to owned UNC test directory"),
    );
    assert!(root.is_absolute() && root.is_dir());
    let dir = tempfile::Builder::new()
        .prefix("imb-c-unc-r2-")
        .tempdir_in(&root)
        .unwrap();
    let src = icc_png(dir.path());
    let off = AtomicBool::new(false);
    // 不論 UNC 路徑長短，Low IL 引擎暫存都必須改放本機 NTFS（2026-11-13 前，
    // 只有長路徑會碰巧走本機短 cwd 分支；短路徑時引擎無法存取暫存而失敗）。
    let job = Job::new_in(dir.path()).unwrap();
    let job_root = job_dir();
    assert!(
        !controlled_temp::is_network_path(&job_root).unwrap(),
        "engine temp stayed on network path: {}",
        job_root.display()
    );
    assert!(!job_root.starts_with(dir.path()));
    let options = Options {
        target_icc: Some(PathBuf::from(
            r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm",
        )),
        ..Default::default()
    };
    let result = convert_with_options_inner(&e, &src, dir.path(), Format::Tiff, &options, &off);
    let ((output, v), warnings) = job.finish_with_warnings(result).unwrap();
    assert!(
        v.icc && v.color_verified && v.exact,
        "{v:?}; warnings={warnings:?}"
    );
    assert!(output.is_file(), "{}", output.display());
    assert_eq!(
        output.parent().unwrap().canonicalize().unwrap(),
        dir.path().canonicalize().unwrap()
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("改用本機受控暫存")
            && warnings[0].contains(&display_path(job_root.parent().unwrap())),
        "{warnings:?}"
    );
    assert!(!job_root.exists());
    // UNC 目錄只應有來源與輸出，沒有 imb-c-work-* 等暫存殘留。
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("imb-c-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(windows)]
#[test]
fn r2_network_temp_root_is_redirected_to_local_ntfs() {
    // 只解析路徑字首，不實際連線；不存在的共用也必須判定為網路路徑。
    let requested = PathBuf::from(r"\\imbc-no-such-host.invalid\share\temp");
    assert!(controlled_temp::is_network_path(&requested).unwrap());
    assert!(
        controlled_temp::is_network_path(&PathBuf::from(
            r"\\?\UNC\imbc-no-such-host.invalid\share\temp"
        ))
        .unwrap()
    );
    let (root, original) = controlled_temp::local_engine_root(&requested).unwrap();
    assert_eq!(original.as_deref(), Some(requested.as_path()));
    assert!(!controlled_temp::is_network_path(&root).unwrap());
    assert!(
        !root
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(r"c:\")
    );
    let local = controlled_temp::preferred_root().unwrap();
    let (same, none) = controlled_temp::local_engine_root(&local).unwrap();
    assert_eq!((same, none), (local, None));
}

#[cfg(windows)]
#[test]
#[ignore = "Owned SMB test directory required; set IMBC_TEST_UNC_ROOT and IMBC_TEST_ENGINE"]
fn r2_unc_input_output_temp_matrix_and_locked_source() {
    use std::os::windows::fs::OpenOptionsExt;
    let e = engine();
    let root = PathBuf::from(std::env::var_os("IMBC_TEST_UNC_ROOT").unwrap());
    assert!(root.is_absolute() && root.is_dir());
    let remote = tempfile::Builder::new()
        .prefix("imb-c-unc-matrix-")
        .tempdir_in(root)
        .unwrap();
    let local = tempfile::tempdir_in(controlled_temp::preferred_root().unwrap()).unwrap();
    let remote_source = icc_png(remote.path());
    let local_source = icc_png(local.path());
    let off = AtomicBool::new(false);
    let expected = sha256(&mut File::open(&local_source).unwrap()).unwrap();
    // 分開 UNC 輸入、輸出、暫存，再測正規化後三者皆為 UNC。
    for (label, input, destination_root, temp) in [
        (
            "input",
            remote_source.clone(),
            local.path().to_owned(),
            local.path().to_owned(),
        ),
        (
            "output",
            local_source.clone(),
            remote.path().to_owned(),
            local.path().to_owned(),
        ),
        (
            "temp",
            local_source.clone(),
            local.path().to_owned(),
            remote.path().to_owned(),
        ),
        (
            "verbatim-all",
            remote_source.canonicalize().unwrap(),
            remote.path().canonicalize().unwrap(),
            remote.path().canonicalize().unwrap(),
        ),
    ] {
        let destination = destination_root.join(label);
        fs::create_dir(&destination).unwrap();
        let job = Job::new_in(&temp).unwrap();
        let owned = job_dir();
        let owned_local = !controlled_temp::is_network_path(&owned).unwrap();
        let result = convert_with_options_inner(
            &e,
            &input,
            &destination,
            Format::Tiff,
            &Options::default(),
            &off,
        );
        let ((output, verification), warnings) = job.finish_with_warnings(result).unwrap();
        eprintln!("UNC_CASE {label}: {verification:?}; warnings={warnings:?}");
        assert!(output.is_file());
        assert!(verification.icc && verification.color_verified && verification.exact);
        assert!(owned_local, "{label}: {}", owned.display());
        if matches!(label, "temp" | "verbatim-all") {
            // 要求的暫存根目錄在 UNC：改用本機受控暫存，並以警告告知實際路徑。
            assert_eq!(warnings.len(), 1, "{label}: {warnings:?}");
            assert!(
                warnings[0].contains("改用本機受控暫存"),
                "{label}: {warnings:?}"
            );
        } else {
            assert!(warnings.is_empty(), "{label}: {warnings:?}");
        }
        assert!(!owned.exists(), "{label}: {}", owned.display());
        assert_eq!(sha256(&mut File::open(&input).unwrap()).unwrap(), expected);
    }
    let destination = remote.path().join("locked-source-output");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("existing.tiff");
    fs::write(&sentinel, b"existing user output").unwrap();
    let hold = File::options()
        .read(true)
        .share_mode(0)
        .open(&remote_source)
        .unwrap();
    let error = convert(&e, &remote_source, &destination, Format::Tiff, &off).unwrap_err();
    assert!(error.contains("開啟來源失敗"), "{error}");
    drop(hold);
    assert_eq!(fs::read(sentinel).unwrap(), b"existing user output");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(
        sha256(&mut File::open(&remote_source).unwrap()).unwrap(),
        expected
    );
}

#[cfg(windows)]
#[test]
fn r2_anonymous_temp_has_exclusive_creation_and_last_handle_cleanup() {
    let job = Job::new().unwrap();
    let root = job_dir();
    let file = job_file().unwrap();
    let entries: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(
        File::open(&entries[0]).is_err(),
        "anonymous temporary data was externally readable"
    );
    let clone = file.try_clone().unwrap();
    drop(file);
    assert!(File::open(&entries[0]).is_err());
    drop(clone);
    assert!(!entries[0].exists());
    job.finish(Ok(())).unwrap();
    assert!(!root.exists());
}
#[test]
#[ignore = "v1.3 runtime"]
fn r2_psb_rejection_is_unchanged_and_writes_nothing() {
    let e = engine();
    let dir = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    for name in ["version2.psb", "version2-disguised.psd"] {
        let input = dir.path().join(name);
        fs::write(&input, b"8BPS\0\x02").unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert!(
            convert_with_options(
                &e,
                &input,
                dir.path(),
                Format::Png,
                &Options::default(),
                &off
            )
            .is_err()
        );
        assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
        assert_eq!(fs::read(input).unwrap(), b"8BPS\0\x02");
    }
}
