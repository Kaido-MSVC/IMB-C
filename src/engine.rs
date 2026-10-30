use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Webp,
    Png,
    Jpeg,
    Avif,
    Psd,
    Tiff,
    Jxl,
}
impl Format {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Png,
            2 => Self::Jpeg,
            3 => Self::Avif,
            4 => Self::Psd,
            5 => Self::Tiff,
            6 => Self::Jxl,
            _ => Self::Webp,
        }
    }
    fn coder(self) -> &'static str {
        match self {
            Self::Webp => "webp:-",
            Self::Png => "png:-",
            Self::Jpeg => "jpeg:-",
            Self::Avif => "avif:-",
            Self::Psd => "psd:-",
            Self::Tiff => "tiff:-",
            Self::Jxl => "jxl:-",
        }
    }
    fn input_coder(self) -> &'static str {
        // Read the saved PSD composite, never interpret layer frames as animation.
        if matches!(self, Self::Psd) {
            "psd:-[0]"
        } else {
            self.coder()
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Avif => "avif",
            Self::Psd => "psd",
            Self::Tiff => "tiff",
            Self::Jxl => "jxl",
        }
    }
    pub fn input(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "webp" => Ok(Self::Webp),
            "png" => Ok(Self::Png),
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "avif" => Ok(Self::Avif),
            "psd" => Ok(Self::Psd),
            "tif" | "tiff" => Ok(Self::Tiff),
            "jxl" => Ok(Self::Jxl),
            _ => Err("僅支援 WebP、PNG、JPEG、AVIF、PSD、TIFF、JXL；不支援 HEIC/HEIF".into()),
        }
    }
}

pub fn locate_engine() -> PathBuf {
    // Development and release both use this copy's private engine.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("ImageMagick").join("magick.exe")))
        .unwrap_or_else(|| PathBuf::from("ImageMagick/magick.exe"))
}

fn private_engine_path(home: &Path) -> Result<OsString> {
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32");
    std::env::join_paths([home, system.as_path()]).map_err(err)
}

fn run(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
) -> Result<()> {
    run_with_timeout(
        engine,
        args,
        source,
        output,
        cancel,
        Duration::from_secs(600),
    )
}

fn run_with_timeout(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    // Only this freshly created random directory is owned by this invocation.
    // Never scan/delete global magick-* files or a directory from a previous run.
    let scratch = tempfile::Builder::new()
        .prefix("imb-c-job-")
        .tempdir()
        .map_err(err)?;
    let scratch_path = scratch.path().to_path_buf();
    let mut stopped = true;
    let result = run_child(
        engine,
        args,
        source,
        output,
        cancel,
        timeout,
        &scratch_path,
        &mut stopped,
    );
    if !stopped {
        // A failed wait cannot establish that the child has released its files.
        // Preserve this directory and report it instead of racing a live child.
        let retained = scratch.keep();
        return Err(format!(
            "{}；無法確認子程序停止，暫存保留於 {}",
            result.unwrap_err(),
            retained.display()
        ));
    }
    match scratch.close() {
        Ok(()) => result,
        Err(e) => Err(format!(
            "{}；暫存清理失敗：{} ({e})",
            result.err().unwrap_or_else(|| "轉換已停止".into()),
            scratch_path.display()
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_child(
    engine: &Path,
    args: &[&str],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
    timeout: Duration,
    scratch: &Path,
    stopped: &mut bool,
) -> Result<()> {
    let stderr = tempfile::tempfile().map_err(err)?;
    let mut command = Command::new(engine);
    command
        .args(args)
        .env("MAGICK_TEMPORARY_PATH", scratch)
        .env("TEMP", scratch)
        .env("TMP", scratch)
        .env("MAGICK_THREAD_LIMIT", "1")
        .env("OMP_NUM_THREADS", "1")
        .stdin(Stdio::from(source.try_clone().map_err(err)?))
        .stdout(Stdio::from(output.try_clone().map_err(err)?))
        .stderr(Stdio::from(stderr.try_clone().map_err(err)?));
    if let Some(home) = engine.parent().filter(|p| p.is_absolute()) {
        // Child-only configuration: never change the user's PATH or installed ImageMagick.
        command
            .current_dir(home)
            .env("PATH", private_engine_path(home)?)
            .env("MAGICK_HOME", home)
            .env("MAGICK_CONFIGURE_PATH", home)
            .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
            .env("MAGICK_FILTER_MODULE_PATH", home.join("modules/filters"))
            .env_remove("MAGICK_CONFIGURE_BUILD_PATH");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("無法啟動 {}：{e}", engine.display()))?;
    *stopped = false;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() > timeout {
            let _ = child.kill();
            child
                .wait()
                .map_err(|e| format!("等待子程序停止失敗：{e}"))?;
            *stopped = true;
            return Err(if cancel.load(Ordering::Relaxed) {
                "已取消".into()
            } else {
                format!("超過單次執行 {} 秒限制", timeout.as_secs())
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                *stopped = true;
                if status.success() {
                    return Ok(());
                }
                let mut stderr = stderr;
                stderr.seek(SeekFrom::Start(0)).map_err(err)?;
                let mut detail = Vec::new();
                stderr.take(4096).read_to_end(&mut detail).map_err(err)?;
                let detail = String::from_utf8_lossy(&detail);
                return Err(format!(
                    "{} 失敗 ({status})：{}",
                    engine.file_name().unwrap_or_default().to_string_lossy(),
                    if detail.trim().is_empty() {
                        "引擎未提供錯誤詳情"
                    } else {
                        detail.trim()
                    }
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                child
                    .wait()
                    .map_err(|wait| format!("{e}；等待子程序停止失敗：{wait}"))?;
                *stopped = true;
                return Err(err(e));
            }
        }
    }
}

// PNG's animation control chunk precedes image data. The ordinary PNG coder
// may expose only a still frame, so reject APNG before passing it to that coder.
fn reject_apng(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(8)).map_err(err)?;
    loop {
        let mut chunk = [0u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            break;
        }
        if &chunk[4..] == b"acTL" {
            return Err("第一階段不支援動畫 PNG".into());
        }
        if &chunk[4..] == b"IDAT" || &chunk[4..] == b"IEND" {
            break;
        }
        let length = u32::from_be_bytes(chunk[..4].try_into().unwrap());
        file.seek(SeekFrom::Current(i64::from(length) + 4))
            .map_err(err)?;
    }
    Ok(())
}

// cICP is a PNG color description independent of ICC. The private conversion
// pipeline cannot yet prove that it survives MIFF and every output coder.
fn png_color_chunks(file: &mut File) -> Result<(Option<[u8; 4]>, bool)> {
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut signature = [0u8; 8];
    file.read_exact(&mut signature).map_err(err)?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return Err("PNG 檔頭無效".into());
    }
    let mut cicp = None;
    let mut iccp = false;
    loop {
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(err)?;
        let length = u32::from_be_bytes(chunk[..4].try_into().unwrap());
        if &chunk[4..] == b"cICP" {
            if length != 4 {
                return Err("PNG cICP 長度無效".into());
            }
            let mut value = [0u8; 4];
            file.read_exact(&mut value).map_err(err)?;
            cicp = Some(value);
            file.seek(SeekFrom::Current(4)).map_err(err)?;
            continue;
        }
        if &chunk[4..] == b"iCCP" {
            iccp = true;
        }
        if &chunk[4..] == b"IDAT" || &chunk[4..] == b"IEND" {
            return Ok((cicp, iccp));
        }
        file.seek(SeekFrom::Current(i64::from(length) + 4))
            .map_err(err)?;
    }
}

fn publish(
    mut temp: NamedTempFile,
    dir: &Path,
    stem: &std::ffi::OsStr,
    format: Format,
) -> Result<PathBuf> {
    temp.as_file().sync_all().map_err(err)?;
    for index in 0..10000 {
        let mut name = OsString::from(stem);
        if index > 0 {
            name.push(format!(" ({index})"));
        }
        name.push(format!(".{}", format.extension()));
        let target = dir.join(name);
        match temp.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temp = e.file;
            }
            Err(e) => return Err(err(e.error)),
        }
    }
    Err("同名檔案過多，無法產生唯一名稱".into())
}

fn image_stage_error(stage: &str, detail: String) -> String {
    // The UI recognizes this exact value as cancellation, not failure.
    if detail == "已取消" {
        return detail;
    }
    let lower = detail.to_ascii_lowercase();
    let hint = if lower.contains("cache resources exhausted")
        || lower.contains("disk allocation failed")
        || lower.contains("no space left")
    {
        "；請檢查暫存磁碟可用空間；每個引擎行程的磁碟快取上限為 8 GiB"
    } else if lower.contains("time limit") || detail.contains("秒限制") {
        "；目前維持每次引擎執行 600 秒上限，請回報圖片尺寸及輸出格式"
    } else {
        ""
    };
    // Keep the original engine diagnostics, including any notices they contain.
    format!("{stage}失敗：{detail}{hint}")
}

#[derive(Clone, Debug)]
pub struct Options {
    pub avif_quality: u8,
    pub webp_lossless: bool,
    pub target_icc: Option<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            avif_quality: 90,
            webp_lossless: true,
            target_icc: None,
        }
    }
}

#[derive(Debug)]
pub struct Verification {
    pub exact: bool,
    pub max_error: f64,
    pub rmse: f64,
    pub input_depth: u32,
    pub output_depth: u32,
    pub icc: bool,
    pub transformed: bool,
}
impl std::fmt::Display for Verification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}; {}→{} bit; ICC {}; max={:.6}; RMSE={:.6}{}",
            if self.exact {
                "pixels exact ✓"
            } else {
                "pixels changed"
            },
            self.input_depth,
            self.output_depth,
            if self.icc {
                "preserved ✓"
            } else {
                "untagged"
            },
            self.max_error,
            self.rmse,
            if self.transformed {
                "; compared after requested color/background conversion"
            } else {
                ""
            }
        )
    }
}

fn invoke(
    engine: &Path,
    args: &[String],
    source: &File,
    output: &File,
    cancel: &AtomicBool,
) -> Result<()> {
    run(
        engine,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        source,
        output,
        cancel,
    )
}

fn metadata(
    engine: &Path,
    coder: &str,
    source: &mut File,
    cancel: &AtomicBool,
) -> Result<(u32, bool, u64, u64)> {
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut out = tempfile::tempfile().map_err(err)?;
    run(
        engine,
        &[
            "identify",
            "-define",
            "png:preserve-iCCP=true",
            "-format",
            "%z\n%[profiles]\n%w\n%h",
            coder,
        ],
        source,
        &out,
        cancel,
    )?;
    out.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut text = String::new();
    out.take(4096).read_to_string(&mut text).map_err(err)?;
    let mut lines = text.lines();
    let depth = lines.next().ok_or("缺少位深資訊")?.parse().map_err(err)?;
    let profiles = lines.next().unwrap_or("");
    let width = lines.next().ok_or("缺少寬度")?.parse().map_err(err)?;
    let height = lines.next().ok_or("缺少高度")?.parse().map_err(err)?;
    Ok((
        depth,
        profiles.split([',', ' ']).any(|s| s == "icc" || s == "icm"),
        width,
        height,
    ))
}

fn extracted(
    engine: &Path,
    coder: &str,
    source: &mut File,
    kind: &str,
    depth: u32,
    cancel: &AtomicBool,
) -> Result<NamedTempFile> {
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut output = tempfile::Builder::new()
        .prefix("imb-c-verify-")
        .tempfile()
        .map_err(err)?;
    let sink = tempfile::tempfile().map_err(err)?;
    let target = format!("{kind}:{}", output.path().display());
    let bits = depth.to_string();
    let mut args = vec!["-define", "png:preserve-iCCP=true", coder];
    if kind == "rgba" {
        args.extend(["-alpha", "on", "-depth", &bits, "-endian", "LSB"]);
        if depth == 32 {
            args.extend(["-define", "quantum:format=floating-point"]);
        }
    }
    args.push(&target);
    run(engine, &args, source, &sink, cancel)?;
    if output.as_file().metadata().map_err(err)?.len() == 0 {
        return Err("驗證資料為空，無法確認畫質".into());
    }
    output.as_file_mut().seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(output)
}

fn sample_error(
    a: &mut File,
    b: &mut File,
    depth: u32,
    expected: u64,
    cancel: &AtomicBool,
) -> Result<(bool, f64, f64)> {
    use std::io::BufReader;
    a.seek(SeekFrom::Start(0)).map_err(err)?;
    b.seek(SeekFrom::Start(0)).map_err(err)?;
    let bytes = if depth == 32 { 4 } else { 2 };
    if a.metadata().map_err(err)?.len() != expected * bytes
        || b.metadata().map_err(err)?.len() != expected * bytes
    {
        return Err("驗證尺寸／像素資料長度不符".into());
    }
    let mut a = BufReader::new(a);
    let mut b = BufReader::new(b);
    let mut aa = [0u8; 65536];
    let mut bb = [0u8; 65536];
    let mut maximum = 0f64;
    let mut sum = 0f64;
    let mut exact = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        // Read exact matching blocks; ordinary files can still return short reads.
        let n = a.read(&mut aa).map_err(err)?;
        if n == 0 {
            break;
        }
        b.read_exact(&mut bb[..n]).map_err(err)?;
        if n % bytes as usize != 0 {
            return Err("驗證樣本長度無效".into());
        }
        for (x, y) in aa[..n]
            .chunks_exact(bytes as usize)
            .zip(bb[..n].chunks_exact(bytes as usize))
        {
            let (x, y) = if depth == 32 {
                (
                    f32::from_le_bytes(x.try_into().unwrap()) as f64,
                    f32::from_le_bytes(y.try_into().unwrap()) as f64,
                )
            } else {
                (
                    u16::from_le_bytes(x.try_into().unwrap()) as f64 / 65535.,
                    u16::from_le_bytes(y.try_into().unwrap()) as f64 / 65535.,
                )
            };
            if !x.is_finite() || !y.is_finite() {
                return Err("驗證遇到非有限浮點像素".into());
            }
            let d = (x - y).abs();
            exact &= d == 0.;
            maximum = maximum.max(d);
            sum += d * d;
        }
    }
    Ok((exact, maximum, (sum / expected.max(1) as f64).sqrt()))
}

#[allow(dead_code)]
pub fn convert(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    // Historical tests/API keep their original lossy WebP behavior.
    let options = Options {
        webp_lossless: false,
        ..Options::default()
    };
    convert_with_options(engine, input, dir, format, &options, cancel).map(|(p, _)| p)
}

pub fn convert_with_options(
    engine: &Path,
    input: &Path,
    dir: &Path,
    format: Format,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<(PathBuf, Verification)> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    let input_format = Format::input(input)?;
    let mut source = File::open(input).map_err(err)?;
    if !source.metadata().map_err(err)?.is_file() {
        return Err("來源不是一般檔案".into());
    }
    if matches!(input_format, Format::Psd) {
        let mut header = [0; 6];
        source.read_exact(&mut header).map_err(err)?;
        if &header != b"8BPS\0\x01" {
            return Err("僅支援 PSD v1，不支援 PSB 或無效 PSD".into());
        }
    }
    if matches!(input_format, Format::Png) {
        reject_apng(&mut source)?;
    }
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut info = tempfile::tempfile().map_err(err)?;
    run(
        engine,
        &[
            "identify",
            "-limit",
            "thread",
            "1",
            "-limit",
            "memory",
            "256MiB",
            "-limit",
            "map",
            "512MiB",
            "-limit",
            "disk",
            "8GiB",
            "-format",
            "%n %w %h\n",
            // Set explicit -ping after -format for metadata-only inspection.
            "-ping",
            input_format.input_coder(),
        ],
        &source,
        &info,
        cancel,
    )
    .map_err(|e| image_stage_error("讀取圖片資訊 (identify)", e))?;
    info.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut frames = String::new();
    info.take(256).read_to_string(&mut frames).map_err(err)?;
    let fields: Vec<_> = frames.split_whitespace().collect();
    if fields.len() != 3 || fields[0] != "1" {
        return Err("僅支援靜態單張圖片（不支援多頁 TIFF 或動畫）".into());
    }
    let width = fields[1].parse::<u64>().map_err(err)?;
    let height = fields[2].parse::<u64>().map_err(err)?;
    if width.saturating_mul(height) > 400_000_000 {
        return Err("圖片超過 400 MP（4 億像素）上限".into());
    }
    if matches!(format, Format::Psd) && (width > 30000 || height > 30000) {
        return Err("PSD 尺寸上限為每邊 30000 像素；不輸出 PSB".into());
    }
    if !(1..=100).contains(&options.avif_quality) {
        return Err("AVIF 品質須為 1–100".into());
    }
    let (input_depth, source_icc, _, _) =
        metadata(engine, input_format.input_coder(), &mut source, cancel)?;
    if matches!(input_format, Format::Png) {
        let (cicp, iccp) = png_color_chunks(&mut source)?;
        if cicp.is_some() && (!iccp || !source_icc) {
            return Err("此 PNG 僅有 cICP、沒有原始 iCCP；目前無法驗證轉檔後完整保留色彩資訊，未保存輸出。請使用已嵌入正確 ICC 的來源".into());
        }
    }
    // CICP-only HDR/wide-gamut AVIF has no ICC for the proven preservation path.
    // Refuse rather than silently relabeling PQ/HLG samples as sRGB.
    if matches!(input_format, Format::Avif) && !source_icc {
        source.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut out = tempfile::tempfile().map_err(err)?;
        run(
            engine,
            &[
                "identify",
                "-format",
                "%[heic:cicp]",
                "-ping",
                input_format.input_coder(),
            ],
            &source,
            &out,
            cancel,
        )?;
        out.seek(SeekFrom::Start(0)).map_err(err)?;
        let mut text = String::new();
        out.take(256).read_to_string(&mut text).map_err(err)?;
        let fields: Vec<u32> = text
            .trim()
            .split('/')
            .filter_map(|x| x.parse().ok())
            .collect();
        if fields.len() >= 2 && ((!matches!(fields[0], 0 | 1 | 2)) || matches!(fields[1], 16 | 18))
        {
            return Err("此 AVIF 僅有 HDR／廣色域 CICP、沒有 ICC；尚未驗證此路徑的完整色彩保留，未保存輸出。請使用已嵌入正確 ICC 的來源".into());
        }
    }
    if options.webp_lossless && matches!(format, Format::Webp) && input_depth > 8 {
        return Err("WebP 僅支援 8-bit；原圖高於 8-bit，無法真正無損。請選 PNG/TIFF/JXL，或關閉 WebP 無損以允許降低位深".into());
    }
    if input_depth > 16 && !matches!(format, Format::Jxl | Format::Tiff) {
        return Err("超過 16-bit／浮點來源請使用 JXL 或 TIFF，避免靜默截斷精度".into());
    }
    let destination = dir.canonicalize().map_err(err)?;
    let mut intermediate = tempfile::tempfile().map_err(err)?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut args: Vec<String> = [
        "-limit",
        "thread",
        "1",
        "-limit",
        "memory",
        "256MiB",
        "-limit",
        "map",
        "512MiB",
        "-limit",
        "disk",
        "8GiB",
        "-define",
        "png:preserve-iCCP=true",
        input_format.input_coder(),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if matches!(input_format, Format::Jpeg) {
        // Apply EXIF Orientation 1..8 once, before writing a new container.
        // This also resets the orientation tag, preventing a second rotation.
        args.push("-auto-orient".into());
    }
    // Snapshot user ICC into a safe generated filename; never interpret a user path as a coder/option.
    let mut profile_snapshot = None;
    let mut target_profile_hash = None;
    if let Some(path) = &options.target_icc {
        if !source_icc {
            return Err("來源沒有 ICC；無法可靠判斷原始色域，請先為來源嵌入正確 ICC".into());
        }
        let mut profile = tempfile::Builder::new()
            .prefix("imb-c-profile-")
            .suffix(".icc")
            .tempfile()
            .map_err(err)?;
        let f = File::open(path).map_err(err)?;
        if f.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
            return Err("ICC 大於 16 MiB".into());
        }
        let mut data = Vec::new();
        f.take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(err)?;
        if data.len() < 132
            || &data[36..40] != b"acsp"
            || u32::from_be_bytes(data[..4].try_into().unwrap()) as usize != data.len()
        {
            return Err("ICC 標頭或長度無效".into());
        }
        use std::io::Write;
        profile.as_file_mut().write_all(&data).map_err(err)?;
        target_profile_hash = Some(sha256(profile.as_file_mut())?);
        args.extend([
            "-profile".into(),
            profile.path().to_string_lossy().into_owned(),
        ]);
        profile_snapshot = Some(profile);
    }
    if matches!(format, Format::Jpeg) {
        args.extend(
            ["-background", "white", "-alpha", "remove", "-alpha", "off"].map(String::from),
        );
    }
    if matches!(format, Format::Psd) {
        args.extend(["+profile", "8bim"].map(String::from));
    }
    args.extend(
        [
            "-evaluate",
            "Add",
            "0",
            "-define",
            "quantum:format=floating-point",
            "-depth",
            "32",
            "miff:-",
        ]
        .map(String::from),
    );
    invoke(engine, &args, &source, &intermediate, cancel)
        .map_err(|e| image_stage_error("圖片轉換 (convert)／高精度中繼", e))?;
    let (_, _, expected_width, expected_height) =
        metadata(engine, "miff:-", &mut intermediate, cancel)?;
    if let Some(expected) = target_profile_hash {
        let mut actual = extracted(engine, "miff:-", &mut intermediate, "icc", 16, cancel)?;
        if sha256(actual.as_file_mut())? != expected {
            return Err("目標 ICC 未正確套用，未保存輸出".into());
        }
    }
    drop(profile_snapshot);
    if intermediate.metadata().map_err(err)?.len() == 0 {
        return Err("中繼輸出為空".into());
    }
    intermediate.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut temp = NamedTempFile::new_in(&destination).map_err(err)?;
    // ICC conversion can produce fractional samples from 8-bit input.
    // Quantize only to 16-bit for formats that support it, matching verification.
    let depth = if input_depth > 16 {
        "32"
    } else if input_depth > 8
        || (options.target_icc.is_some()
            && matches!(format, Format::Png | Format::Tiff | Format::Jxl | Format::Psd))
        || (matches!(input_format, Format::Psd)
            && matches!(format, Format::Png | Format::Tiff | Format::Jxl))
    {
        "16"
    } else {
        "8"
    };
    let q = options.avif_quality.to_string();
    let mut args = vec![
        "-limit", "thread", "1", "-limit", "memory", "256MiB", "-limit", "map", "512MiB", "-limit",
        "disk", "8GiB", "miff:-", "-depth", depth,
    ];
    if input_depth > 16 {
        args.extend(["-define", "quantum:format=floating-point"]);
    }
    if matches!(format, Format::Jpeg) {
        args.extend(["-quality", "90"]);
    }
    if matches!(format, Format::Webp) {
        if options.webp_lossless {
            args.extend([
                "-quality",
                "100",
                "-define",
                "webp:lossless=true",
                "-define",
                "webp:exact=true",
            ]);
        } else {
            args.extend(["-quality", "90", "-define", "webp:lossless=false"]);
        }
    }
    if matches!(format, Format::Avif) {
        args.extend([
            "-quality",
            &q,
            "-define",
            "heic:speed=6",
            "-define",
            "heic:chroma=444",
        ]);
    }
    if matches!(format, Format::Png) {
        args.extend(["-define", "png:preserve-iCCP=true"]);
        if depth == "16" {
            args.extend(["-define", "png:bit-depth=16"]);
        }
    }
    if matches!(format, Format::Tiff) {
        args.extend(["-compress", "Zip", "-define", "tiff:alpha=unassociated"]);
    }
    if matches!(format, Format::Jxl) {
        args.extend(["-define", "jxl:distance=0", "-quality", "100"]);
    }
    if matches!(format, Format::Psd) {
        args.extend(["-define", "psd:write-layers=true"]);
    }
    args.push(format.coder());
    run(engine, &args, &intermediate, temp.as_file(), cancel)
        .map_err(|e| image_stage_error("圖片轉換 (convert)", e))?;
    if temp.as_file().metadata().map_err(err)?.len() == 0 {
        return Err("轉換輸出為空".into());
    }
    let (output_depth, output_icc, output_width, output_height) =
        metadata(engine, format.input_coder(), temp.as_file_mut(), cancel)?;
    if (output_width, output_height) != (expected_width, expected_height) {
        return Err("驗證輸出寬高不符，未保存輸出".into());
    }
    let transformed = options.target_icc.is_some()
        || matches!(format, Format::Jpeg)
        || matches!(input_format, Format::Jpeg);
    let mut reference = if transformed {
        intermediate.try_clone().map_err(err)?
    } else {
        source.try_clone().map_err(err)?
    };
    let reference_coder = if transformed {
        "miff:-"
    } else {
        input_format.input_coder()
    };
    let (_, expected_icc, _, _) = metadata(engine, reference_coder, &mut reference, cancel)?;
    // libjxl's decoder may expose an ICC representation of its internal color
    // encoding even when the source and encoded JXL contain no ICC profile.
    let jxl_decoder_profile = matches!(format, Format::Jxl) && !expected_icc && output_icc;
    if expected_icc != output_icc && !jxl_decoder_profile {
        return Err(if expected_icc {
            "ICC 描述檔遺失，未保存輸出".into()
        } else {
            "輸出意外新增 ICC 描述檔，未保存輸出".into()
        });
    }
    if expected_icc {
        let mut a = extracted(engine, reference_coder, &mut reference, "icc", 16, cancel)?;
        let mut b = extracted(
            engine,
            format.input_coder(),
            temp.as_file_mut(),
            "icc",
            16,
            cancel,
        )?;
        if sha256(a.as_file_mut())? != sha256(b.as_file_mut())? {
            return Err("ICC 描述檔不一致，未保存輸出".into());
        }
    }
    let check_depth = if input_depth > 16 { 32 } else { 16 };
    let mut a = extracted(
        engine,
        reference_coder,
        &mut reference,
        "rgba",
        check_depth,
        cancel,
    )?;
    let mut b = extracted(
        engine,
        format.input_coder(),
        temp.as_file_mut(),
        "rgba",
        check_depth,
        cancel,
    )?;
    let (exact, max_error, rmse) = sample_error(
        a.as_file_mut(),
        b.as_file_mut(),
        check_depth,
        expected_width * expected_height * 4,
        cancel,
    )?;
    let lossless = matches!(format, Format::Png | Format::Tiff | Format::Jxl)
        || (matches!(format, Format::Webp) && options.webp_lossless);
    if lossless && !exact {
        return Err(format!("無損像素驗證失敗 max={max_error:.8}，未保存輸出"));
    }
    if lossless && input_depth > 8 && output_depth < input_depth {
        return Err("輸出位深降低，未保存無損輸出".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    let result = publish(
        temp,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        format,
    )?;
    Ok((
        result,
        Verification {
            exact,
            max_error,
            rmse,
            input_depth,
            output_depth,
            icc: expected_icc,
            transformed,
        },
    ))
}

pub fn sha256(file: &mut File) -> Result<String> {
    use sha2::{Digest, Sha256};
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    Ok(format!("{:x}", hash.finalize()))
}

// JPEG reconstruction is deliberately separate from ordinary pixel conversion.
// A source snapshot and a private output file prevent races with destination names.
pub fn jpeg_reconstruction(
    tools: &Path,
    input: &Path,
    dir: &Path,
    wrap: bool,
    reference: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, String, bool)> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    let ext = input
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if (wrap && ext != "jpg" && ext != "jpeg") || (!wrap && ext != "jxl") {
        return Err("此工具僅接受 JPEG → JXL 封裝或 JXL → 原始 JPEG 還原".into());
    }
    let mut source = tempfile::tempfile().map_err(err)?;
    let mut original = File::open(input).map_err(err)?;
    std::io::copy(&mut original, &mut source).map_err(err)?;
    source.seek(SeekFrom::Start(0)).map_err(err)?;
    let destination = dir.canonicalize().map_err(err)?;
    let mut output = NamedTempFile::new_in(&destination).map_err(err)?;
    let cjxl = tools.join("cjxl.exe");
    let djxl = tools.join("djxl.exe");
    let hash;
    let verified;
    if wrap {
        hash = sha256(&mut source)?;
        run(
            &cjxl,
            &["-", "-", "--lossless_jpeg=1", "--num_threads=1"],
            &source,
            output.as_file(),
            cancel,
        )?;
        output.as_file_mut().seek(SeekFrom::Start(0)).map_err(err)?;
        let mut restored = tempfile::tempfile().map_err(err)?;
        run(
            &djxl,
            &[
                "-",
                "-",
                "--output_format=jpg",
                "--reconstruct_jpeg",
                "--num_threads=1",
            ],
            output.as_file(),
            &restored,
            cancel,
        )?;
        if sha256(&mut restored)? != hash {
            return Err("JPEG 往返 SHA-256 不一致，未保存輸出".into());
        }
        verified = true;
    } else {
        run(
            &djxl,
            &[
                "-",
                "-",
                "--output_format=jpg",
                "--reconstruct_jpeg",
                "--num_threads=1",
            ],
            &source,
            output.as_file(),
            cancel,
        )?;
        hash = sha256(output.as_file_mut())?;
        if let Some(path) = reference {
            if sha256(&mut File::open(path).map_err(err)?)? != hash {
                return Err("與參考 JPEG 的 SHA-256 不一致，未保存輸出".into());
            }
            verified = true;
        } else {
            verified = false;
        }
    }
    if output.as_file().metadata().map_err(err)?.len() == 0 {
        return Err("輸出為空".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    let result = publish(
        output,
        &destination,
        input.file_stem().ok_or("來源檔名無效")?,
        if wrap { Format::Jxl } else { Format::Jpeg },
    )?;
    Ok((result, hash, verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn verification_detects_changed_empty_and_float_samples() {
        use std::io::Write;
        let mut a = tempfile::tempfile().unwrap();
        let mut b = tempfile::tempfile().unwrap();
        a.write_all(&[0, 0, 255, 255]).unwrap();
        b.write_all(&[0, 0, 0, 0]).unwrap();
        let (exact, max, rmse) =
            sample_error(&mut a, &mut b, 16, 2, &AtomicBool::new(false)).unwrap();
        assert!(!exact);
        assert_eq!(max, 1.);
        assert!((rmse - 0.5f64.sqrt()).abs() < 1e-10);
        assert!(sample_error(&mut a, &mut b, 16, 3, &AtomicBool::new(false)).is_err());
        a.set_len(0).unwrap();
        b.set_len(0).unwrap();
        a.seek(SeekFrom::Start(0)).unwrap();
        b.seek(SeekFrom::Start(0)).unwrap();
        for x in [-0.25f32, 2.0] {
            a.write_all(&x.to_le_bytes()).unwrap();
            b.write_all(&x.to_le_bytes()).unwrap();
        }
        assert!(
            sample_error(&mut a, &mut b, 32, 2, &AtomicBool::new(false))
                .unwrap()
                .0
        );
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn v13_float_precision() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("float.tiff");
        fs::write(&src, include_bytes!("../tests/fixtures/v13-float32.tiff")).unwrap();
        for format in [Format::Tiff, Format::Jxl] {
            let (_, v) = convert_with_options(
                &engine,
                &src,
                dir.path(),
                format,
                &Options::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(v.exact && v.input_depth == 32 && v.output_depth == 32);
        }
        assert!(
            convert_with_options(
                &engine,
                &src,
                dir.path(),
                Format::Png,
                &Options::default(),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("浮點")
        );
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn v13_cicp_only_hdr_rejected() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("cicp.avif");
        fs::write(&src, include_bytes!("../tests/fixtures/v13-cicp-pq.avif")).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert!(
            convert_with_options(
                &engine,
                &src,
                dir.path(),
                Format::Jxl,
                &Options::default(),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("CICP")
        );
        assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
        let png = dir.path().join("sdr.png");
        fs::write(&png, include_bytes!("../tests/fixtures/v13-hidden.png")).unwrap();
        let (avif, _) = convert_with_options(
            &engine,
            &png,
            dir.path(),
            Format::Avif,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let (_, v) = convert_with_options(
            &engine,
            &avif,
            dir.path(),
            Format::Png,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(v.exact);
    }
    #[test]
    fn png_cicp_parser_finds_color_metadata() {
        let mut f = tempfile::tempfile().unwrap();
        f.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        f.write_all(&4u32.to_be_bytes()).unwrap();
        f.write_all(b"cICP").unwrap();
        f.write_all(&[9, 16, 0, 1]).unwrap();
        f.write_all(&0u32.to_be_bytes()).unwrap();
        assert_eq!(png_color_chunks(&mut f).unwrap(), (Some([9, 16, 0, 1]), false));
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn cicp_only_hdr_png_is_rejected_without_output() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("pq.png");
        fs::write(&input, include_bytes!("../tests/fixtures/v13-cicp-only-pq.png")).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        for format in [Format::Png, Format::Tiff, Format::Jxl, Format::Avif] {
            let error = convert_with_options(
                &engine, &input, dir.path(), format,
                &Options::default(), &AtomicBool::new(false),
            ).unwrap_err();
            assert!(error.contains("cICP"), "{error}");
            assert_eq!(count, fs::read_dir(dir.path()).unwrap().count());
        }
    }
    #[test]
    #[ignore = "Requires v1.3 runtime; set IMBC_TEST_ENGINE"]
    fn jpeg_exif_orientations_to_avif() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let original = include_bytes!("../tests/fixtures/acceptance-v13/baseline-metadata.jpg");
        for orientation in 1..=8u8 {
            let mut bytes = original.to_vec();
            let tag = bytes.windows(4).position(|x| x == [0x12, 0x01, 0x03, 0x00]).unwrap();
            bytes[tag + 8] = orientation;
            let input = dir.path().join(format!("orientation-{orientation}.jpg"));
            fs::write(&input, bytes).unwrap();
            let (avif, v) = convert_with_options(
                &engine, &input, dir.path(), Format::Avif,
                &Options::default(), &AtomicBool::new(false),
            ).unwrap();
            let mut output = File::open(avif).unwrap();
            let (_, _, w, h) = metadata(&engine, "avif:-", &mut output, &AtomicBool::new(false)).unwrap();
            assert_eq!((w, h), if orientation >= 5 { (13, 17) } else { (17, 13) });
            assert!(v.max_error < 0.15, "orientation {orientation}: {v}");
            let mut raw = File::open(&input).unwrap();
            let mut a = extracted(&engine, "jpeg:-", &mut raw, "rgba", 16, &AtomicBool::new(false)).unwrap();
            let mut b = extracted(&engine, "avif:-", &mut output, "rgba", 16, &AtomicBool::new(false)).unwrap();
            let mut source_pixels = Vec::new();
            let mut output_pixels = Vec::new();
            a.read_to_end(&mut source_pixels).unwrap();
            b.read_to_end(&mut output_pixels).unwrap();
            let mut error = 0u64;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let (sx, sy) = match orientation {
                        1 => (x, y), 2 => (16 - x, y),
                        3 => (16 - x, 12 - y), 4 => (x, 12 - y),
                        5 => (y, x), 6 => (y, 12 - x),
                        7 => (16 - y, 12 - x), 8 => (16 - y, x),
                        _ => unreachable!(),
                    };
                    for channel in 0..3 {
                        let source_index = (sy * 17 + sx) * 8 + channel * 2;
                        let output_index = (y * w as usize + x) * 8 + channel * 2;
                        let from = u16::from_le_bytes(source_pixels[source_index..source_index+2].try_into().unwrap());
                        let to = u16::from_le_bytes(output_pixels[output_index..output_index+2].try_into().unwrap());
                        error += from.abs_diff(to) as u64;
                    }
                }
            }
            assert!(error as f64 / (w * h * 3) as f64 / 65535. < 0.15,
                "orientation {orientation}: pixel positions differ");
        }
    }
    #[test]
    fn stage_diagnostics_preserve_details_and_cancellation() {
        assert_eq!(image_stage_error("identify", "已取消".into()), "已取消");
        let original = "cache resources exhausted; License: unchanged notice";
        let message = image_stage_error("identify", original.into());
        assert!(message.contains(original));
        assert!(message.contains("8 GiB"));
        assert!(image_stage_error("convert", "超過單次執行 600 秒限制".into()).contains("600 秒"));
    }

    #[test]
    #[ignore = "Requires v1.2.1 candidate engine; set IMBC_TEST_ENGINE"]
    fn v121_small_formats_and_failure_stages() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("透明圖 [1] %03d.png");
        let bytes = include_bytes!("../tests/fixtures/rgba-v12.png");
        fs::write(&source, bytes).unwrap();
        let off = AtomicBool::new(false);
        for format in [
            Format::Png,
            Format::Jpeg,
            Format::Webp,
            Format::Avif,
            Format::Psd,
            Format::Tiff,
            Format::Jxl,
        ] {
            let converted = convert(&engine, &source, dir.path(), format, &off).unwrap();
            let restored = convert(&engine, &converted, dir.path(), Format::Png, &off).unwrap();
            assert!(fs::metadata(restored).unwrap().len() > 0);
        }
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let bad = dir.path().join("broken.png");
        fs::write(&bad, b"broken").unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        let error = convert(&engine, &bad, dir.path(), Format::Png, &off).unwrap_err();
        assert!(error.contains("identify"), "{error}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);

        // A tiny PNG with an invalid zlib stream but a valid IHDR and chunk CRCs.
        // Metadata inspection must succeed; only the actual conversion decodes IDAT.
        let corrupt = dir.path().join("invalid-idat.png");
        fs::write(
            &corrupt,
            include_bytes!("../tests/fixtures/invalid-idat-v121.png"),
        )
        .unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        let error = convert(&engine, &corrupt, dir.path(), Format::Png, &off).unwrap_err();
        assert!(error.contains("convert"), "{error}");
        assert!(!error.contains("identify"), "{error}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
        let error = convert(
            &engine,
            &dir.path().join("invalid-idat.png"),
            dir.path(),
            Format::Png,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error, "已取消");
    }
    #[test]
    fn heic_and_heif_are_rejected_before_engine_launch() {
        for ext in ["heic", "HEIF", "psb"] {
            assert!(Format::input(Path::new(&format!("image.{ext}"))).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn owned_scratch_cleanup_on_success_failure_cancel_and_timeout() {
        let peer = tempfile::Builder::new()
            .prefix("imb-c-job-")
            .tempdir()
            .unwrap();
        let sentinel = peer.path().join("magick-do-not-delete");
        fs::write(&sentinel, b"unrelated data").unwrap();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let parent_temp = std::env::var_os("TEMP");
        for mode in ["success", "failure", "cancel", "timeout"] {
            let input = tempfile::tempfile().unwrap();
            let mut output = tempfile::tempfile().unwrap();
            let cancel = AtomicBool::new(false);
            let action = match mode {
                "success" => "exit 0",
                "failure" => "exit 7",
                _ => "Start-Sleep -Seconds 30",
            };
            let script = format!(
                "$ErrorActionPreference='Stop'; if ($env:TEMP -ne $env:MAGICK_TEMPORARY_PATH -or $env:TMP -ne $env:TEMP) {{ throw 'temp mismatch' }}; [IO.File]::WriteAllText((Join-Path $env:TEMP 'magick-fixture'),'private image'); [Console]::Out.WriteLine($env:TEMP); [Console]::Out.Flush(); {action}"
            );
            let result = std::thread::scope(|scope| {
                if mode == "cancel" {
                    let observed = output.try_clone().unwrap();
                    let cancel = &cancel;
                    scope.spawn(move || {
                        let start = Instant::now();
                        while observed.metadata().unwrap().len() == 0
                            && start.elapsed() < Duration::from_secs(8)
                        {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        cancel.store(true, Ordering::Relaxed);
                    });
                }
                run_with_timeout(
                    &shell,
                    &["-NoProfile", "-NonInteractive", "-Command", &script],
                    &input,
                    &output,
                    &cancel,
                    if mode == "timeout" {
                        Duration::from_secs(3)
                    } else {
                        Duration::from_secs(10)
                    },
                )
            });
            output.seek(SeekFrom::Start(0)).unwrap();
            let mut recorded = String::new();
            output.read_to_string(&mut recorded).unwrap();
            let scratch = PathBuf::from(recorded.trim());
            assert!(
                scratch.is_absolute(),
                "child did not reach scratch fixture: {mode}: {result:?}"
            );
            assert!(
                !scratch.exists(),
                "scratch left after {mode}: {}",
                scratch.display()
            );
            assert_eq!(result.is_ok(), mode == "success", "{mode}: {result:?}");
            assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated data");
            assert_eq!(std::env::var_os("TEMP"), parent_temp);
        }
    }
    #[test]
    fn publish_never_overwrites_even_when_racing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("image.png"), b"original").unwrap();
        std::thread::scope(|s| {
            for _ in 0..8 {
                let p = dir.path();
                s.spawn(move || {
                    let mut t = NamedTempFile::new_in(p).unwrap();
                    t.write_all(b"new").unwrap();
                    publish(t, p, std::ffi::OsStr::new("image"), Format::Png).unwrap();
                });
            }
        });
        assert_eq!(fs::read(dir.path().join("image.png")).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 9);
    }
    #[test]
    fn cancelled_job_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            convert(
                Path::new("missing.exe"),
                Path::new("missing.png"),
                dir.path(),
                Format::Png,
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .contains("取消")
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn active_cancellation_terminates_child() {
        let cancel = AtomicBool::new(false);
        let input = tempfile::tempfile().unwrap();
        let output = tempfile::tempfile().unwrap();
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(500));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(
                run(
                    &shell,
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "Start-Sleep -Seconds 30"
                    ],
                    &input,
                    &output,
                    &cancel
                )
                .unwrap_err()
                .contains("取消")
            );
        });
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    fn inspect(engine: &Path) -> Command {
        let home = engine.parent().unwrap();
        let mut cmd = Command::new(engine);
        cmd.current_dir(home)
            .env("PATH", private_engine_path(home).unwrap())
            .env("MAGICK_HOME", home)
            .env("MAGICK_CONFIGURE_PATH", home)
            .env("MAGICK_CODER_MODULE_PATH", home.join("modules/coders"))
            .env("MAGICK_FILTER_MODULE_PATH", home.join("modules/filters"))
            .env_remove("MAGICK_CONFIGURE_BUILD_PATH");
        cmd
    }
    fn pixel(engine: &Path, path: &Path) -> Vec<f64> {
        let r = inspect(engine)
            .args([
                "identify",
                "-format",
                "%[opaque] %[fx:p{0,0}.r] %[fx:p{0,0}.g] %[fx:p{0,0}.b] %[fx:p{0,0}.a]",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let text = String::from_utf8(r.stdout).unwrap();
        let mut fields = text.split_whitespace();
        let opaque = fields.next().unwrap().eq_ignore_ascii_case("true");
        let mut values: Vec<f64> = fields.map(|s| s.parse().unwrap()).collect();
        if opaque {
            values[3] = 1.0;
        }
        values
    }
    #[test]
    #[ignore = "Requires packaged v1.1 engine"]
    fn psd_composite_and_transparency() {
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let off = AtomicBool::new(false);
        let psd = dir.path().join("layered.psd");
        fs::write(&psd, include_bytes!("../tests/fixtures/layered.psd")).unwrap();
        let out = convert(&engine, &psd, dir.path(), Format::Png, &off).unwrap();
        let p = pixel(&engine, &out);
        assert!(
            p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01,
            "must use RED saved composite, not blue/green layers: {p:?}"
        );
        let no = dir.path().join("no-composite.psd");
        fs::write(&no, include_bytes!("../tests/fixtures/no-composite.psd")).unwrap();
        assert!(convert(&engine, &no, dir.path(), Format::Png, &off).is_err());
        let alpha = dir.path().join("alpha.png");
        fs::write(&alpha, include_bytes!("../tests/fixtures/alpha.png")).unwrap();
        for format in [
            Format::Png,
            Format::Webp,
            Format::Avif,
            Format::Psd,
            Format::Jpeg,
        ] {
            let file = convert(&engine, &alpha, dir.path(), format, &off).unwrap();
            let raster = convert(&engine, &file, dir.path(), Format::Png, &off).unwrap();
            let p = pixel(&engine, &raster);
            if matches!(format, Format::Jpeg) {
                assert!((p[3] - 1.0).abs() < 0.01, "JPEG must be opaque: {p:?}");
                assert!(
                    p[0] > 0.55 && p[1] > 0.7 && p[2] > 0.9,
                    "JPEG white background: {p:?}"
                );
            } else {
                assert!(
                    (p[3] - 128.0 / 255.0).abs() < 0.03,
                    "alpha lost: {format:?} {p:?}"
                );
                assert!(
                    (p[0] - 40.0 / 255.0).abs() < 0.08
                        && (p[1] - 130.0 / 255.0).abs() < 0.08
                        && (p[2] - 220.0 / 255.0).abs() < 0.08,
                    "color changed: {format:?} {p:?}"
                );
            }
        }
    }
    #[test]
    #[ignore = "Requires ImageMagick; IMBC_TEST_ENGINE selects the packaged engine"]
    fn imagemagick_matrix_and_failures() {
        let engine = std::env::var_os("IMBC_TEST_ENGINE")
            .map(PathBuf::from)
            .unwrap_or_else(locate_engine);
        assert!(engine.is_file());
        let dir = tempfile::tempdir().unwrap();
        let off = AtomicBool::new(false);
        let seed = dir.path().join("原圖 [1] %03d 空白.png");
        fs::write(&seed, include_bytes!("../tests/fixtures/seed.png")).unwrap();
        let original = fs::read(&seed).unwrap();
        let mut sources = vec![seed.clone()];
        for format in [Format::Webp, Format::Jpeg, Format::Avif, Format::Psd] {
            sources.push(convert(&engine, &seed, dir.path(), format, &off).unwrap());
        }
        for source in sources {
            let bytes = fs::read(&source).unwrap();
            for format in [
                Format::Webp,
                Format::Png,
                Format::Jpeg,
                Format::Avif,
                Format::Psd,
            ] {
                let output = convert(&engine, &source, dir.path(), format, &off).unwrap();
                assert_ne!(source, output);
                assert!(fs::metadata(&output).unwrap().len() > 0);
                let result = inspect(&engine)
                    .arg("identify")
                    .args(["-format", "%m %wx%h"])
                    .stdin(Stdio::from(File::open(&output).unwrap()))
                    .arg(format.input_coder())
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "inspect {format:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8_lossy(&result.stdout),
                    format!(
                        "{} 8x6",
                        match format {
                            Format::Webp => "WEBP",
                            Format::Png => "PNG",
                            Format::Jpeg => "JPEG",
                            Format::Avif => "AVIF",
                            Format::Psd => "PSD",
                            Format::Tiff => "TIFF",
                            Format::Jxl => "JXL",
                        }
                    )
                );
            }
            assert_eq!(fs::read(&source).unwrap(), bytes);
        }
        assert_eq!(fs::read(&seed).unwrap(), original);
        let bad = dir.path().join("bad.png");
        fs::write(&bad, b"broken").unwrap();
        let before = fs::read_dir(dir.path()).unwrap().count();
        assert!(convert(&engine, &bad, dir.path(), Format::Png, &off).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), before);
        let animation = dir.path().join("animation.webp");
        fs::write(
            &animation,
            include_bytes!("../tests/fixtures/animation.webp"),
        )
        .unwrap();
        assert!(
            convert(&engine, &animation, dir.path(), Format::Png, &off)
                .unwrap_err()
                .contains("靜態")
        );
        assert!(
            convert(
                Path::new("missing.exe"),
                &seed,
                dir.path(),
                Format::Png,
                &off
            )
            .is_err()
        );
        let apng = dir.path().join("animation.png");
        fs::write(&apng, b"\x89PNG\r\n\x1a\n\0\0\0\x08acTL\0\0\0\x02\0\0\0\0").unwrap();
        assert!(
            convert(&engine, &apng, dir.path(), Format::Png, &off)
                .unwrap_err()
                .contains("動畫")
        );
    }
}
