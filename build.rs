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
    // GUI／CLI 必須以靜態 CRT 建置：程式本身不載入可再散布的 Microsoft DLL，
    // 隨包的 Microsoft DLL 只由 ImageMagick 引擎使用（.cargo/config.toml 設定；
    // 若以 RUSTFLAGS 覆寫 config，需自行加入 -C target-feature=+crt-static）。
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && !env::var("CARGO_CFG_TARGET_FEATURE")
            .unwrap_or_default()
            .split(',')
            .any(|f| f == "crt-static")
    {
        panic!("IMB-C requires -C target-feature=+crt-static on MSVC (see .cargo/config.toml)");
    }
    // 靜態匯入只搜尋 System32，排除應用程式目錄、CWD 與 PATH。
    println!("cargo:rustc-link-arg=/DEPENDENTLOADFLAG:0x800");
    // GUI 與 CLI 都附相同產品版本；原始檔名各自正確，避免 CLI 缺少版本資源。
    for binary in ["imb-c", "imb-c-cli"] {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(format!("{binary}.res"));
        let mut command = Command::new(&rc);
        command.current_dir("assets").arg("/nologo");
        if binary == "imb-c-cli" {
            command.arg("/dIMBC_CLI");
        }
        assert!(
            command
                .arg(format!("/fo{}", out.display()))
                .arg("app.rc")
                .status()
                .expect("Run Windows SDK resource compiler")
                .success()
        );
        println!("cargo:rustc-link-arg-bin={binary}={}", out.display());
    }
}
