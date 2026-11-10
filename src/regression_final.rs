use super::*;
use std::fs;
fn engine() -> PathBuf {
    PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap())
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/icc-final")
        .join(name)
}
fn profile(e: &Path, p: &Path) -> Vec<u8> {
    let mut f = File::open(p).unwrap();
    let mut out = extracted(
        e,
        Format::input(p).unwrap().input_coder(),
        &mut f,
        "icc",
        16,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut bytes = Vec::new();
    out.read_to_end(&mut bytes).unwrap();
    bytes
}
fn rgb_target() -> PathBuf {
    PathBuf::from(r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm")
}
#[test]
#[ignore = "v1.3 runtime"]
fn final_jxl_gray_color_is_not_misreported_as_proven_embedded_icc() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    for name in ["gray-enumerated.jxl", "gray-embedded.jxl"] {
        let src = fixture(name);
        let before = fs::read(&src).unwrap();
        let original_profile = profile(&e, &src);
        assert_eq!(&original_profile[16..20], b"GRAY");
        if name == "gray-embedded.jxl" {
            assert_eq!(
                original_profile,
                fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/icc-r3/gray-gamma22.icc")
                )
                .unwrap()
            );
        }
        for lossless in [true, false] {
            let options = Options {
                webp_lossless: lossless,
                ..Default::default()
            };
            let count = fs::read_dir(d.path()).unwrap().count();
            let err =
                convert_with_options(&e, &src, d.path(), Format::Webp, &options, &off).unwrap_err();
            assert!(err.starts_with("[IMBC_WEBP_JXL_COLOR]GRAY"), "{err}");
            assert!(
                err.contains("合成") && err.contains("不能斷定") && err.contains("關閉"),
                "{err}"
            );
            assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
            assert_eq!(options.webp_lossless, lossless);
        }
        let (png, v) =
            convert_with_options(&e, &src, d.path(), Format::Png, &Options::default(), &off)
                .unwrap();
        assert!(v.exact && v.icc && v.color_verified);
        assert_eq!(profile(&e, &png), original_profile);
        assert_eq!(fs::read(src).unwrap(), before);
    }
}
#[test]
#[ignore = "v1.3 runtime"]
fn final_jxl_rgb_decoder_profile_still_preserves_true_lossless_webp() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let src = fixture("rgb-enumerated.jxl");
    let before = fs::read(&src).unwrap();
    let original_profile = profile(&e, &src);
    assert_eq!(&original_profile[16..20], b"RGB ");
    let (out, v) = convert_with_options(
        &e,
        &src,
        d.path(),
        Format::Webp,
        &Options::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(v.exact && v.icc && v.color_verified && !v.transformed);
    assert_eq!(profile(&e, &out), original_profile);
    assert_eq!(fs::read(src).unwrap(), before);
}
#[test]
#[ignore = "v1.3 runtime"]
fn final_gray_to_rgb_webp_precision_rejection_and_explicit_lossy_path() {
    let e = engine();
    let d = tempfile::tempdir().unwrap();
    let off = AtomicBool::new(false);
    for src in [
        fixture("gray-enumerated.jxl"),
        fixture("gray-embedded.jxl"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icc-r3/gray-gray.png"),
    ] {
        let before = fs::read(&src).unwrap();
        let options = Options {
            target_icc: Some(rgb_target()),
            ..Default::default()
        };
        let count = fs::read_dir(d.path()).unwrap().count();
        let err =
            convert_with_options(&e, &src, d.path(), Format::Webp, &options, &off).unwrap_err();
        assert!(err.starts_with("[IMBC_WEBP_RGB_PRECISION]max="), "{err}");
        assert!(
            err.contains("無損像素驗證失敗") && err.contains("8-bit") && err.contains("關閉"),
            "{err}"
        );
        assert_eq!(count, fs::read_dir(d.path()).unwrap().count());
        assert!(options.webp_lossless);
        let options = Options {
            webp_lossless: false,
            ..options
        };
        let (out, v) =
            convert_with_options(&e, &src, d.path(), Format::Webp, &options, &off).unwrap();
        assert!(v.icc && v.color_verified && v.transformed && !v.exact);
        assert_eq!(profile(&e, &out), fs::read(rgb_target()).unwrap());
        assert_eq!(fs::read(src).unwrap(), before);
    }
}
