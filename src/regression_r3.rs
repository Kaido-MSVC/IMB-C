use super::*;
use std::fs;
fn engine() -> PathBuf {
    PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap())
}
fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/icc-r3")
            .join(name),
    )
    .unwrap()
}
fn output_profile(e: &Path, p: &Path) -> Vec<u8> {
    let mut f = File::open(p).unwrap();
    let coder = Format::input(p).unwrap().input_coder();
    let mut out = extracted(e, coder, &mut f, "icc", 16, &AtomicBool::new(false)).unwrap();
    let mut b = Vec::new();
    out.read_to_end(&mut b).unwrap();
    b
}
#[test]
#[ignore = "v1.3 runtime"]
fn r3_rgb_icc_png_storage_types_are_detected_and_preserved() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    let expected = fixture("rgb-existing.icc");
    for name in ["rgb-rgb.png", "gray-rgb.png"] {
        let src = d.path().join(name);
        let original = fixture(name);
        fs::write(&src, &original).unwrap();
        for fmt in [Format::Png, Format::Webp, Format::Jxl] {
            if name == "gray-rgb.png" && matches!(fmt, Format::Jxl) {
                continue;
            }
            let (out, v) = convert_with_options(&e, &src, d.path(), fmt, &Options::default(), &off)
                .unwrap_or_else(|err| panic!("{name} -> {fmt:?}: {err}"));
            assert!(v.icc && v.color_verified && v.exact);
            assert_eq!(output_profile(&e, &out), expected);
            if matches!(fmt, Format::Png) {
                assert_eq!(
                    fs::read(&out).unwrap()[25],
                    6,
                    "RGB ICC output must use RGB-compatible PNG storage"
                );
            }
        }
        assert_eq!(fs::read(&src).unwrap(), original);
    }
}
#[test]
#[ignore = "v1.3 runtime"]
fn r3_gray_png_rgb_icc_jxl_replacement_is_rejected() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gray-rgb.png");
    fs::write(&src, fixture("gray-rgb.png")).unwrap();
    let count = fs::read_dir(d.path()).unwrap().count();
    let err = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Jxl,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(err.contains("ICC 描述檔不一致"), "{err}");
    assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
}
#[test]
#[ignore = "v1.3 runtime"]
fn r3_gray_icc_webp_requires_explicit_rgb_conversion() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gray-gray.png");
    let bytes = fixture("gray-gray.png");
    fs::write(&src, &bytes).unwrap();
    let count = fs::read_dir(d.path()).unwrap().count();
    let err = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Webp,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        err.contains("ICC") && err.contains("WebP") && err.contains("GRAY"),
        "{err}"
    );
    assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
    assert_eq!(fs::read(src).unwrap(), bytes);
}
#[test]
#[ignore = "v1.3 runtime"]
fn r3_gray_icc_png_and_explicit_rgb_target_preserve_contract() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gray-gray.png");
    fs::write(&src, fixture("gray-gray.png")).unwrap();
    let off = AtomicBool::new(false);
    let (out, v) =
        convert_with_options(&e, &src, d.path(), Format::Png, &Options::default(), &off).unwrap();
    assert!(v.icc && v.exact);
    assert_eq!(output_profile(&e, &out), fixture("gray-gamma22.icc"));
    let (jxl, v) =
        convert_with_options(&e, &src, d.path(), Format::Jxl, &Options::default(), &off).unwrap();
    assert!(v.icc && v.exact);
    assert_eq!(output_profile(&e, &jxl), fixture("gray-gamma22.icc"));
    let target = d.path().join("rgb.icc");
    fs::write(&target, fixture("rgb-existing.icc")).unwrap();
    let opt = Options {
        target_icc: Some(target),
        ..Default::default()
    };
    let (out, v) = convert_with_options(&e, &src, d.path(), Format::Png, &opt, &off).unwrap();
    assert!(v.icc && v.exact && v.transformed);
    assert_eq!(output_profile(&e, &out), fixture("rgb-existing.icc"));
}

#[test]
#[ignore = "v1.3 runtime"]
fn r3_unreadable_embedded_iccp_is_never_treated_as_untagged() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("unreadable.png");
    let bytes = fixture("unreadable-gray-iccp.png");
    fs::write(&p, &bytes).unwrap();
    for fmt in [Format::Png, Format::Jxl, Format::Webp] {
        let count = fs::read_dir(d.path()).unwrap().count();
        let err = convert_with_options(
            &e,
            &p,
            d.path(),
            fmt,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(err.contains("iCCP") && err.contains("ICC"), "{err}");
        assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
    }
    assert_eq!(fs::read(p).unwrap(), bytes);
}

#[test]
#[ignore = "v1.3 runtime"]
fn r3_gray_webp_explicit_rgb_lossy_conversion_has_rgb_profile() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gray-gray.png");
    let original = fixture("gray-gray.png");
    fs::write(&src, &original).unwrap();
    let gray = d.path().join("gray.icc");
    fs::write(&gray, fixture("gray-gamma22.icc")).unwrap();
    let options = Options {
        target_icc: Some(gray),
        ..Default::default()
    };
    let count = fs::read_dir(d.path()).unwrap().count();
    let err = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Webp,
        &options,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(err.contains("GRAY") && err.contains("WebP"), "{err}");
    assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
    let target = d.path().join("rgb.icc");
    fs::write(&target, fixture("rgb-existing.icc")).unwrap();
    let options = Options {
        target_icc: Some(target),
        webp_lossless: false,
        ..Default::default()
    };
    let (out, v) = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Webp,
        &options,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(v.icc && v.color_verified && v.transformed);
    let profile = output_profile(&e, &out);
    assert_eq!(&profile[16..20], b"RGB ");
    assert_eq!(profile, fixture("rgb-existing.icc"));
    assert_eq!(fs::read(src).unwrap(), original);
}
#[test]
#[ignore = "v1.3 runtime"]
fn r3_plain_untagged_png_does_not_dilute_real_color_warnings() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("plain.png");
    fs::write(&src, include_bytes!("../tests/fixtures/v13-hidden.png")).unwrap();
    let (_, v) = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Png,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!v.icc && !v.color_missing && v.warnings.is_empty());
    assert!(!v.to_string().contains("not verified"));
}
