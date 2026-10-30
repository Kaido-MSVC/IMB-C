#[path = "../engine.rs"]
mod engine;
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
fn main() {
    let root=PathBuf::from(std::env::args_os().nth(1).expect("root"));
    let e=root.join("target/release/ImageMagick/magick.exe");
    let out=root.join("work/focused");fs::create_dir_all(&out).unwrap();
    let fixtures=root.join("tests/fixtures/acceptance-v13");
    let off=AtomicBool::new(false);let defaults=engine::Options::default();
    let options=engine::Options{target_icc:Some(PathBuf::from(r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm")),..Default::default()};
    for name in ["icc8.png","pq-original.jxl","hlg-original.jxl"] {
        for f in [engine::Format::Png,engine::Format::Tiff,engine::Format::Jxl,engine::Format::Psd] {
            let (p,v)=engine::convert_with_options(&e,&fixtures.join(name),&out,f,if name=="icc8.png"{&options}else{&defaults},&off).unwrap_or_else(|x|panic!("{name} {f:?}: {x}"));
            assert!(v.exact && v.icc,"{name}: {v}");println!("PASS {name} {f:?}: {v}; {}",p.display());
        }
    }
    let tools=root.join("target/release/JXL");
    for name in ["baseline-metadata.jpg","progressive-metadata.jpg"] {
        let src=fixtures.join(name);let before=fs::read(&src).unwrap();
        let (packed,h,ok)=engine::jpeg_reconstruction(&tools,&src,&out,true,None,&off).unwrap();assert!(ok);
        let (restored,h2,ok)=engine::jpeg_reconstruction(&tools,&packed,&out,false,Some(&src),&off).unwrap();assert!(ok);
        assert_eq!(h,h2);assert_eq!(before,fs::read(restored).unwrap());assert_eq!(before,fs::read(src).unwrap());
        println!("PASS JPEG bytes {name} length={} SHA256={h}; EXIF/XMP/ICC/COM included",before.len());
    }
}
