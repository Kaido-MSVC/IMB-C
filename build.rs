use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.png");
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rustc-env=SLINT_EMBED_RESOURCES=true");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let rc = env::var_os("RC").map(PathBuf::from).unwrap_or_else(|| {
        let root = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK required"))
            .join("Windows Kits/10/bin");
        let mut candidates: Vec<_> = std::fs::read_dir(root)
            .expect("Install Windows SDK")
            .filter_map(Result::ok)
            .map(|x| x.path().join("x64/rc.exe"))
            .filter(|x| x.is_file())
            .collect();
        candidates.sort();
        candidates.pop().expect("rc.exe not found; set RC")
    });
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app.res");
    assert!(
        Command::new(rc)
            .current_dir("assets")
            .arg("/nologo")
            .arg(format!("/fo{}", out.display()))
            .arg("app.rc")
            .status()
            .expect("Run Windows SDK resource compiler")
            .success()
    );
    println!("cargo:rustc-link-arg-bin=imb-c={}", out.display());
}
