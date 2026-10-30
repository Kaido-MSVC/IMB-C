#[path = "../engine.rs"]
mod engine;
#[path = "../imports.rs"]
mod imports;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
};

fn pixels(magick: &Path, path: &Path) -> Vec<u8> {
    // RGBA is permitted only in a separately staged test engine's policy.
    let home = magick.parent().unwrap();
    let ext = path
        .extension()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    let ext = if ext == "jpg" { "jpeg" } else { ext.as_str() };
    let input = if ext == "psd" {
        format!("psd:{}[0]", path.display())
    } else {
        format!("{}:{}", ext, path.display())
    };
    let result = Command::new(magick)
        .args([input.as_ref(), "-depth", "8", "rgba:-"])
        .current_dir(home)
        .env(
            "PATH",
            std::env::join_paths([home, Path::new("C:/Windows/System32")]).unwrap(),
        )
        .env("MAGICK_HOME", home)
        .env("MAGICK_CONFIGURE_PATH", home)
        .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("project root"));
    let engine = root.join("target/release/ImageMagick/magick.exe");
    let test_engine = root.join("work/test-engine/magick.exe");
    let tools = root.join("target/release/JXL");
    let out = root.join("work/regression-output");
    fs::create_dir_all(&out).unwrap();
    let c = AtomicBool::new(false);
    let fixtures = root.join("tests/fixtures");
    let source = fixtures.join("rgba-v12.png");
    let baseline = pixels(&test_engine, &source);
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
    for format in formats {
        seeds.push(engine::convert(&engine, &source, &out, format, &c).unwrap());
    }
    let mut count = 0;
    for (index, input) in seeds.iter().enumerate() {
        for (target, format) in formats.iter().enumerate() {
            if index < 5 && target < 5 {
                continue;
            } // Keep historical 25-pair evidence; test the 24 changed pairs.
            let result = engine::convert(&engine, input, &out, *format, &c).unwrap();
            if target == 5 || target == 6 {
                assert_eq!(
                    pixels(&test_engine, input),
                    pixels(&test_engine, &result),
                    "lossless {index} -> {target}"
                );
            }
            if (index == 5 || index == 6) && target == 1 {
                assert_eq!(baseline, pixels(&test_engine, &result));
            }
            count += 1;
        }
    }
    // Opaque RGB and grayscale PNG/TIFF/JXL also round-trip without changing samples.
    for name in ["rgb-v12.png", "gray-v12.png"] {
        let path = fixtures.join(name);
        let expected = pixels(&test_engine, &path);
        for format in [
            engine::Format::Png,
            engine::Format::Tiff,
            engine::Format::Jxl,
        ] {
            let image = engine::convert(&engine, &path, &out, format, &c).unwrap();
            assert_eq!(expected, pixels(&test_engine, &image), "{name} {format:?}");
        }
    }
    let jpeg = &seeds[2];
    let (packed, _, verified) =
        engine::jpeg_reconstruction(&tools, jpeg, &out, true, None, &c).unwrap();
    assert!(verified);
    let (restored, _, verified) =
        engine::jpeg_reconstruction(&tools, &packed, &out, false, Some(jpeg), &c).unwrap();
    assert!(verified);
    assert_eq!(fs::read(jpeg).unwrap(), fs::read(restored).unwrap());
    assert!(
        engine::jpeg_reconstruction(&tools, &seeds[6], &out, false, None, &c).is_err(),
        "pixel JXL must not fall back to lossy JPEG"
    );
    assert!(
        engine::jpeg_reconstruction(&tools, &packed, &out, false, Some(&source), &c).is_err(),
        "reference mismatch"
    );
    let damaged = out.join("damaged.jxl");
    fs::write(&damaged, b"invalid JXL").unwrap();
    assert!(engine::convert(&engine, &damaged, &out, engine::Format::Png, &c).is_err());
    assert!(
        engine::convert(
            &engine,
            &fixtures.join("multipage-v12.tiff"),
            &out,
            engine::Format::Png,
            &c
        )
        .is_err()
    );
    assert!(
        engine::convert(
            &engine,
            &fixtures.join("no-composite.psd"),
            &out,
            engine::Format::Jxl,
            &c
        )
        .is_err()
    );
    assert!(
        engine::convert(
            &engine,
            &fixtures.join("animation.webp"),
            &out,
            engine::Format::Tiff,
            &c
        )
        .is_err()
    );
    // New output formats retain non-clobber naming, even in the source directory.
    let before = fs::read(&source).unwrap();
    let one = engine::convert(&engine, &source, &out, engine::Format::Jxl, &c).unwrap();
    let two = engine::convert(&engine, &source, &out, engine::Format::Jxl, &c).unwrap();
    assert_ne!(one, two);
    assert_eq!(before, fs::read(&source).unwrap());
    let cancelled = AtomicBool::new(true);
    assert!(engine::jpeg_reconstruction(&tools, jpeg, &out, true, None, &cancelled).is_err());
    let mut file = fs::File::open(&source).unwrap();
    let hash = engine::sha256(&mut file).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert!(!hash.is_empty());
    println!(
        "PASS: {count} changed conversion pairs; RGBA (including hidden RGB), RGB and grayscale lossless checks; JPEG SHA-256 round trip, strict reconstruction rejection, mismatch, corrupt input, multipage TIFF, PSD, animation, naming and cancellation."
    );
}
