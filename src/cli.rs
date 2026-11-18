//! 無 GUI 的批次入口，轉檔與驗證共用核心引擎。
use crate::{engine, iat, imports};
use serde_json::json;
use std::{
    ffi::OsString,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

const HELP: &str = "IMB-C command line\n\
Usage: imb-c-cli convert <input>... --output-dir <folder> --format <format> [options]\n\
Formats: webp png jpeg avif psd psb tiff jxl\n\
Options:\n  --input <file>           Add an input (repeatable)\n  --input-dir <folder>     Snapshot a folder\n  --recursive              Include subfolders (skip directory links)\n  --mode lossless|lossy    Explicit encoding mode\n  --quality 1..100         JXL lossy: 1..99\n  --target-icc <file>      Explicit color conversion\n  --iat <absolute path>    Optional external validator; never searched in PATH\n  --no-iat                 Disable optional external validation\n  --report-json <new file> Write a new summary; never overwrite\n  --help / --version\n\
The bundled engine uses Microsoft Visual C++ Runtime DLLs; their terms are in\n\
RUNTIME-RECIPIENT-TERMS.md in this package.\n\
Sources are read-only. Output names are numbered to avoid overwriting.\n\
JSON lines on stdout; diagnostics on stderr.\n\
Exit: 0=conversions completed, 1=conversion failures, 2=arguments/environment/report error, 130=cancelled.\n\
iat verdicts do not change conversion status or remove files; explicit tool failures return 2.\n";

static CANCEL: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
unsafe extern "system" fn console_event(event: u32) -> i32 {
    if matches!(event, 0 | 1 | 2 | 5 | 6) {
        CANCEL.store(true, Ordering::Relaxed);
        i32::from(event <= 1) // Other events permit immediate Windows termination.
    } else {
        0
    }
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
struct Console;
impl Console {
    fn new() -> Self {
        CANCEL.store(false, Ordering::Relaxed);
        #[cfg(windows)]
        // SAFETY: console_event has the required ABI and only accesses a static atomic.
        unsafe {
            SetConsoleCtrlHandler(Some(console_event), 1);
        }
        Self
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: unregister the same static handler installed by new.
        unsafe {
            SetConsoleCtrlHandler(Some(console_event), 0);
        }
    }
}

#[derive(Debug)]
struct Args {
    inputs: Vec<PathBuf>,
    folders: Vec<PathBuf>,
    output: PathBuf,
    format: engine::Format,
    options: engine::Options,
    recursive: bool,
    iat: Option<PathBuf>,
    no_iat: bool,
    report: Option<PathBuf>,
}
fn parse(args: Vec<OsString>) -> Result<Args, String> {
    let mut args = args.into_iter();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("convert")) {
        return Err("expected convert; use --help".into());
    }
    let (mut inputs, mut folders, mut output, mut format, mut mode, mut quality) =
        (Vec::new(), Vec::new(), None, None, None, 90u8);
    let (mut recursive, mut iat, mut no_iat, mut report, mut target_icc) =
        (false, None, false, None, None);
    let mut quality_set = false;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if positional {
            inputs.push(PathBuf::from(arg));
            continue;
        }
        let Some(key) = arg.to_str() else {
            inputs.push(PathBuf::from(arg));
            continue;
        };
        if key == "--" {
            positional = true;
            continue;
        }
        if key == "--recursive" {
            recursive = true;
            continue;
        }
        if key == "--no-iat" {
            no_iat = true;
            continue;
        }
        if !key.starts_with('-') {
            inputs.push(PathBuf::from(arg));
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        match key {
            "--input" => inputs.push(PathBuf::from(value)),
            "--input-dir" => folders.push(PathBuf::from(value)),
            "--output-dir" => output = Some(PathBuf::from(value)),
            "--target-icc" => target_icc = Some(PathBuf::from(value)),
            "--iat" => iat = Some(PathBuf::from(value)),
            "--report-json" => report = Some(PathBuf::from(value)),
            "--format" => {
                format = Some(match value.to_str().ok_or("invalid format")? {
                    "webp" => engine::Format::Webp,
                    "png" => engine::Format::Png,
                    "jpeg" | "jpg" => engine::Format::Jpeg,
                    "avif" => engine::Format::Avif,
                    "psd" => engine::Format::Psd,
                    "psb" => engine::Format::Psb,
                    "tiff" | "tif" => engine::Format::Tiff,
                    "jxl" => engine::Format::Jxl,
                    _ => return Err("unsupported format".into()),
                })
            }
            "--mode" => {
                mode = Some(match value.to_str() {
                    Some("lossless") => true,
                    Some("lossy") => false,
                    _ => return Err("mode must be lossless or lossy".into()),
                })
            }
            "--quality" => {
                quality_set = true;
                quality = value
                    .to_str()
                    .and_then(|s| s.parse::<u8>().ok())
                    .filter(|q| (1..=100).contains(q))
                    .ok_or("quality must be 1..100")?
            }
            _ => return Err(format!("unknown option {key}")),
        }
    }
    let format = format.ok_or("missing --format")?;
    let lossless = mode.unwrap_or(!matches!(
        format,
        engine::Format::Avif | engine::Format::Jpeg
    ));
    if (lossless && matches!(format, engine::Format::Jpeg))
        || (!lossless
            && matches!(
                format,
                engine::Format::Png
                    | engine::Format::Tiff
                    | engine::Format::Psd
                    | engine::Format::Psb
            ))
    {
        return Err("encoding mode not supported by this format".into());
    }
    if lossless && quality_set {
        return Err("--quality is not valid in lossless mode".into());
    }
    if no_iat && iat.is_some() {
        return Err("--iat conflicts with --no-iat".into());
    }
    if matches!(format, engine::Format::Jxl) && !lossless && quality == 100 {
        return Err("JXL lossy quality must be 1..99".into());
    }
    if inputs.is_empty() && folders.is_empty() {
        return Err("missing input".into());
    }
    Ok(Args {
        inputs,
        folders,
        output: output.ok_or("missing --output-dir")?,
        format,
        recursive,
        iat,
        no_iat,
        report,
        options: engine::Options {
            avif_quality: quality,
            avif_lossless: lossless,
            jxl_quality: quality,
            jxl_lossless: lossless,
            lossy_quality: quality,
            webp_lossless: lossless,
            target_icc,
        },
    })
}

#[allow(dead_code)] // GUI dispatch; separate console binary always uses CLI.
pub fn is_cli_invocation(args: &[OsString]) -> bool {
    args.first()
        .is_some_and(|a| a == "convert" || a == "--help" || a == "--version")
}

fn help_requested(args: &[OsString]) -> bool {
    args.iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--help")
}
fn write_info(mut output: impl Write, text: &str) -> i32 {
    match output
        .write_all(text.as_bytes())
        .and_then(|_| output.flush())
    {
        Ok(()) => 0,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => 0,
        Err(_) => 2,
    }
}
fn diagnostic(text: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}
fn path_hex(path: &std::path::Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .map(|c| format!("{c:04x}"))
        .collect()
}
/// Windows subsystem 入口沒有可用串流時，以視窗回報說明或失敗。
#[allow(dead_code)] // Only GUI binary calls this entry.
pub fn run_from_gui(args: Vec<OsString>) -> i32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            title: *const u16,
            kind: u32,
        ) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX);
    }
    let output = unsafe { GetStdHandle(-11i32 as u32) };
    let no_output = output.is_null() || output as isize == -1;
    let help = help_requested(&args);
    let version = args.len() == 1 && args[0] == "--version";
    let code = if no_output && (help || version) {
        0
    } else {
        run(args)
    };
    if no_output {
        let text = if help {
            HELP.to_owned()
        } else if version {
            format!("IMB-C {}", env!("CARGO_PKG_VERSION"))
        } else {
            format!(
                "CLI exit code: {code}\nUse imb-c-cli.exe in a terminal for JSON reports and diagnostics."
            )
        };
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "IMB-C".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                if code == 0 { 0x40 } else { 0x10 },
            );
        }
    }
    code
}

/// 回傳明確退出碼，呼叫前不建立任何 GUI 元件。
pub fn run(args: Vec<OsString>) -> i32 {
    if help_requested(&args) {
        return write_info(std::io::stdout().lock(), HELP);
    }
    if args.len() == 1 && args[0] == "--version" {
        return write_info(
            std::io::stdout().lock(),
            &format!("IMB-C {}\n", env!("CARGO_PKG_VERSION")),
        );
    }
    let _console = Console::new();
    let result = execute(args);
    engine::shutdown_instance();
    match result {
        Ok(code) => code,
        Err(e) => {
            diagnostic(&format!("IMB-C: {e}"));
            2
        }
    }
}
fn execute(args: Vec<OsString>) -> Result<i32, String> {
    let mut args = parse(args)?;
    if !args.output.is_dir() {
        return Err("output directory must exist".into());
    }
    if let Some(report) = &args.report
        && (report.exists()
            || !report
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."))
                .is_dir())
    {
        return Err("report must be a new file in an existing directory".into());
    }
    for folder in &args.folders {
        let (files, warnings) = imports::folder(folder, args.recursive)?;
        args.inputs.extend(files);
        for w in warnings {
            diagnostic(&w);
        }
    }
    args.inputs.sort();
    args.inputs.dedup();
    if args.inputs.is_empty() {
        return Err("no supported inputs".into());
    }
    let application = std::env::current_exe().map_err(|e| e.to_string())?;
    let iat_requested = !args.no_iat && args.iat.is_some();
    let (iat, unavailable) = if iat_requested {
        match iat::Client::discover(args.iat.as_deref(), &application) {
            Ok(client) => (Some(client), None),
            Err(reason) => {
                diagnostic(&format!("iat 驗證不可用（{reason}）"));
                (None, Some(iat::Outcome::unavailable(reason)))
            }
        }
    } else {
        (None, None)
    };
    let iat_available = iat.is_some();
    let mut iat_failed = unavailable.is_some();
    let engine = engine::locate_engine();
    for warning in engine::recover_stale_temp() {
        diagnostic(&warning);
    }
    let mut results = Vec::new();
    let mut failures = 0;
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    for input in &args.inputs {
        if CANCEL.load(Ordering::Relaxed) {
            break;
        }
        let row = match engine::convert_with_options(
            &engine,
            input,
            &args.output,
            args.format,
            &args.options,
            &CANCEL,
        ) {
            Ok((output, v)) => {
                let external = iat
                    .as_ref()
                    .map(|client| client.compare(input, &output, &CANCEL))
                    .or_else(|| unavailable.clone());
                if external.as_ref().is_some_and(iat::Outcome::is_tool_failure) {
                    iat_failed = true;
                }
                json!({"input":engine::display_path(input),"input_wtf16_hex":path_hex(input),"output":engine::display_path(&output),"output_wtf16_hex":path_hex(&output),"status":"completed","lossless_requested":v.lossless_requested,"composite_decoded":v.composite_decoded,"transformed":v.transformed,"pixels_exact":v.exact,"input_depth":v.input_depth,"output_depth":v.output_depth,"icc_verified":v.color_verified,"warnings":v.warnings,"iat_requested":iat_requested,"iat_available":iat_available,"iat":external.map(|o|o.json(0))})
            }
            Err(e) if e == "已取消" => {
                CANCEL.store(true, Ordering::Relaxed);
                break;
            }
            Err(e) => {
                failures += 1;
                json!({"input":engine::display_path(input),"input_wtf16_hex":path_hex(input),"status":"failed","error":e,"iat_requested":iat_requested,"iat_available":iat_available,"iat":unavailable.as_ref().map(|o|o.json(0))})
            }
        };
        serde_json::to_writer(&mut stdout, &row).map_err(|e| e.to_string())?;
        writeln!(&mut stdout).map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
        if args.report.is_some() {
            results.push(row);
        }
    }
    let code = if CANCEL.load(Ordering::Relaxed) {
        130
    } else if failures > 0 {
        1
    } else if iat_failed {
        2
    } else {
        0
    };
    if let Some(report) = args.report {
        let parent = report
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(
            &mut temp,
            &json!({"version":env!("CARGO_PKG_VERSION"),"exit_code":code,"iat_requested":iat_requested,"iat_available":iat_available,"results":results}),
        )
        .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist_noclobber(report).map_err(|e| e.to_string())?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<OsString> {
        s.iter().map(OsString::from).collect()
    }
    #[test]
    fn help_anywhere_before_separator_and_broken_output_do_not_panic() {
        assert!(help_requested(&args(&["convert", "a.png", "--help"])));
        assert!(!help_requested(&args(&["convert", "--", "--help"])));
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(write_info(Closed, HELP), 0);
        let mut output = Vec::new();
        assert_eq!(write_info(&mut output, HELP), 0);
        assert!(output.starts_with(b"IMB-C"));
    }
    #[test]
    fn surrogate_positional_paths_and_json_roundtrip_are_lossless() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let raw = OsString::from_wide(&[0xd800, 46, 112, 110, 103]);
        let mut a = args(&["convert"]);
        a.push(raw.clone());
        a.extend(args(&["--format", "png", "--output-dir", "out"]));
        let a = parse(a).unwrap();
        assert_eq!(a.inputs[0].as_os_str(), &raw);
        let encoded = path_hex(&a.inputs[0]);
        let units: Vec<_> = encoded
            .as_bytes()
            .chunks(4)
            .map(|c| u16::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(units, raw.encode_wide().collect::<Vec<_>>());
    }
    #[test]
    fn gui_dispatch_ignores_files_and_unknown_launch_flags() {
        for a in ["convert", "--help", "--version"] {
            assert!(is_cli_invocation(&args(&[a])));
        }
        for a in ["image.png", "--unknown", "C:\\images\\a.png"] {
            assert!(!is_cli_invocation(&args(&[a])));
        }
        assert!(!is_cli_invocation(&[]));
    }
    #[test]
    fn lossless_quality_and_conflicting_iat_options_are_errors() {
        for extra in [
            vec!["--quality", "90"],
            vec!["--iat", "D:/iat.exe", "--no-iat"],
        ] {
            let mut a = args(&["convert", "a.png", "--output-dir", "out", "--format", "png"]);
            a.extend(args(&extra));
            assert!(parse(a).is_err());
        }
    }
    #[test]
    fn non_unicode_report_paths_are_display_only() {
        use std::os::windows::ffi::OsStringExt;
        let raw = OsString::from_wide(&[67, 58, 92, 0xd800, 46, 112, 110, 103]);
        let path = PathBuf::from(raw.clone());
        assert!(serde_json::to_string(&path).is_err());
        let row = json!({"input": engine::display_path(&path)});
        assert!(serde_json::to_string(&row).is_ok());
        assert_eq!(path.as_os_str(), &raw);
    }
    #[test]
    fn close_signals_set_cancel_without_claiming_graceful_shutdown() {
        for event in [0, 1, 2, 5, 6] {
            CANCEL.store(false, Ordering::Relaxed);
            assert_eq!(unsafe { console_event(event) }, i32::from(event <= 1));
            assert!(CANCEL.load(Ordering::Relaxed));
        }
        CANCEL.store(false, Ordering::Relaxed);
    }
    #[test]
    fn modes_and_bad_arguments() {
        let a = parse(args(&[
            "convert",
            "a.png",
            "--output-dir",
            "out",
            "--format",
            "avif",
            "--mode",
            "lossless",
        ]))
        .unwrap();
        assert!(a.options.avif_lossless);
        for s in [
            vec![
                "convert",
                "a",
                "--format",
                "jpeg",
                "--mode",
                "lossless",
                "--output-dir",
                "out",
            ],
            vec![
                "convert",
                "a",
                "--format",
                "jxl",
                "--mode",
                "lossy",
                "--quality",
                "100",
                "--output-dir",
                "out",
            ],
            vec!["convert", "--unknown", "x"],
        ] {
            assert!(parse(args(&s)).is_err());
        }
    }
}
