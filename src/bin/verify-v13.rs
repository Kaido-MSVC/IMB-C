#[path = "../engine.rs"]
mod engine;
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("project root"))
        .canonicalize()
        .unwrap();
    let engine = root.join("target/release/ImageMagick/magick.exe");
    let tools = root.join("target/release/JXL");
    let f = root.join("tests/fixtures");
    let out = root.join("work/v13-regression");
    fs::create_dir_all(&out).unwrap();
    let c = AtomicBool::new(false);
    let opt = engine::Options::default();
    let mut rows = Vec::new();
    let formats = [
        engine::Format::Webp,
        engine::Format::Png,
        engine::Format::Jpeg,
        engine::Format::Avif,
        engine::Format::Psd,
        engine::Format::Tiff,
        engine::Format::Jxl,
    ];
    let mut seeds = Vec::new();
    if !std::env::args().any(|a| a == "--tail") {
        for format in formats {
            let (p, v) = engine::convert_with_options(
                &engine,
                &f.join("v13-hidden.png"),
                &out,
                format,
                &opt,
                &c,
            )
            .unwrap();
            println!("PASS hidden RGBA {format:?}: {v}");
            rows.push(format!("hidden {format:?}: {v}"));
            seeds.push(p);
        }
        for src in &seeds {
            for format in formats {
                let result = engine::convert_with_options(&engine, src, &out, format, &opt, &c);
                if src.extension().unwrap() == "psd" && matches!(format, engine::Format::Webp) {
                    assert!(result.unwrap_err().contains("無損像素驗證失敗"));
                    rows.push(
                        "PSD fractional composite → lossless WebP: correctly rejected".into(),
                    );
                    continue;
                }
                let (_, v) =
                    result.unwrap_or_else(|e| panic!("{} -> {format:?}: {e}", src.display()));
                rows.push(format!("{} → {format:?}: {v}", src.display()));
            }
        }
        println!("PASS 49 format pairs");
        for n in ["v13-icc16.png", "v13-Rec2100PQ.png", "v13-Rec2100HLG.png"] {
            for format in [
                engine::Format::Png,
                engine::Format::Tiff,
                engine::Format::Jxl,
                engine::Format::Psd,
            ] {
                let (_, v) =
                    engine::convert_with_options(&engine, &f.join(n), &out, format, &opt, &c)
                        .unwrap();
                assert!(v.exact && v.icc && v.input_depth == 16 && v.output_depth >= 16);
                rows.push(format!("{n} {format:?}: {v}"));
                println!("PASS {n} {format:?}");
            }
            let before = fs::read_dir(&out).unwrap().count();
            assert!(
                engine::convert_with_options(
                    &engine,
                    &f.join(n),
                    &out,
                    engine::Format::Webp,
                    &opt,
                    &c
                )
                .unwrap_err()
                .contains("8-bit")
            );
            assert_eq!(before, fs::read_dir(&out).unwrap().count());
        }
    } else {
        let (p, _) = engine::convert_with_options(
            &engine,
            &f.join("v13-hidden.png"),
            &out,
            engine::Format::Jpeg,
            &opt,
            &c,
        )
        .unwrap();
        let (j, _) = engine::convert_with_options(
            &engine,
            &f.join("v13-hidden.png"),
            &out,
            engine::Format::Jxl,
            &opt,
            &c,
        )
        .unwrap();
        seeds = vec![p.clone(), p.clone(), p.clone(), p.clone(), p.clone(), p, j];
    }
    // Quality changes must alter actual encoded output. Both remain classified as lossy.
    let a = engine::Options {
        avif_quality: 20,
        ..opt.clone()
    };
    let b = engine::Options {
        avif_quality: 90,
        ..opt.clone()
    };
    let (low, _) = engine::convert_with_options(
        &engine,
        &f.join("rgba-v12.png"),
        &out,
        engine::Format::Avif,
        &a,
        &c,
    )
    .unwrap();
    let (high, _) = engine::convert_with_options(
        &engine,
        &f.join("rgba-v12.png"),
        &out,
        engine::Format::Avif,
        &b,
        &c,
    )
    .unwrap();
    assert_ne!(fs::read(low).unwrap(), fs::read(high).unwrap());
    // ICC conversion uses an embedded source profile and a separately selected target.
    let target = root.join("work/v13-target.icc");
    let profile =
        PathBuf::from(r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm");
    fs::copy(profile, &target).unwrap();
    let color = engine::Options {
        target_icc: Some(target.clone()),
        ..opt.clone()
    };
    let (_, v) = engine::convert_with_options(
        &engine,
        &f.join("v13-Rec2100PQ.png"),
        &out,
        engine::Format::Tiff,
        &color,
        &c,
    )
    .unwrap();
    assert!(v.transformed && v.icc && v.exact);
    assert!(
        engine::convert_with_options(
            &engine,
            &f.join("v13-hidden.png"),
            &out,
            engine::Format::Png,
            &color,
            &c
        )
        .unwrap_err()
        .contains("沒有 ICC")
    );
    fs::write(&target, b"bad ICC").unwrap();
    assert!(
        engine::convert_with_options(
            &engine,
            &f.join("v13-Rec2100PQ.png"),
            &out,
            engine::Format::Tiff,
            &color,
            &c
        )
        .is_err()
    );
    let jpeg = &seeds[2];
    let (packed, _, verified) =
        engine::jpeg_reconstruction(&tools, jpeg, &out, true, None, &c).unwrap();
    assert!(verified);
    let (restored, _, verified) =
        engine::jpeg_reconstruction(&tools, &packed, &out, false, Some(jpeg), &c).unwrap();
    assert!(verified);
    assert_eq!(fs::read(jpeg).unwrap(), fs::read(restored).unwrap());
    assert!(engine::jpeg_reconstruction(&tools, &seeds[6], &out, false, None, &c).is_err());
    assert!(
        engine::jpeg_reconstruction(
            &tools,
            &packed,
            &out,
            false,
            Some(&f.join("v13-hidden.png")),
            &c
        )
        .is_err()
    );
    for n in [
        "invalid-idat-v121.png",
        "animation.webp",
        "multipage-v12.tiff",
        "no-composite.psd",
    ] {
        let count = fs::read_dir(&out).unwrap().count();
        assert!(
            engine::convert_with_options(&engine, &f.join(n), &out, engine::Format::Jxl, &opt, &c)
                .is_err(),
            "{n}"
        );
        assert_eq!(count, fs::read_dir(&out).unwrap().count());
    }
    assert!(
        engine::convert_with_options(
            &engine,
            &f.join("v13-hidden.png"),
            &out,
            engine::Format::Png,
            &opt,
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .contains("取消")
    );
    if !rows.is_empty() {fs::write(root.join("work/V13-RESULTS.txt"), rows.join("\n")).unwrap();}
    println!(
        "PASS: quality, ICC conversion/rejection, 16-bit WebP rejection, JPEG byte roundtrip/mismatch/strict rejection, corrupt/animated/multipage/PSD/cancel safety"
    );
}
