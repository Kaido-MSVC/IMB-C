use std::{
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub language: i32,
    pub theme: i32,
    pub format: i32,
    pub source_folder: PathBuf,
    pub output_folder: PathBuf,
    pub avif_quality: i32,
    pub webp_lossless: bool,
    pub notify: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: 0,
            theme: 0,
            format: 0,
            source_folder: PathBuf::new(),
            output_folder: PathBuf::new(),
            avif_quality: 90,
            webp_lossless: true,
            notify: true,
        }
    }
}
const LANGUAGES: [&str; 4] = ["zh-TW", "zh-CN", "en", "ja"];
const THEMES: [&str; 3] = ["system", "light", "dark"];
pub fn path() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("IMBC_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("IMB-C")))
        .ok_or_else(|| std::io::Error::other("LOCALAPPDATA unavailable"))?;
    Ok(base.join("preferences.ini"))
}
fn hex(path: &Path) -> String {
    path.to_string_lossy()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn unhex(s: &str) -> PathBuf {
    let bytes: Option<Vec<u8>> = s
        .as_bytes()
        .chunks(2)
        .map(|c| {
            if c.len() == 2 {
                std::str::from_utf8(c)
                    .ok()
                    .and_then(|v| u8::from_str_radix(v, 16).ok())
            } else {
                None
            }
        })
        .collect();
    bytes
        .and_then(|b| String::from_utf8(b).ok())
        .map(PathBuf::from)
        .unwrap_or_default()
}
fn parse(text: &str) -> Preferences {
    let mut p = Preferences::default();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "language" => {
                    p.language = LANGUAGES.iter().position(|s| *s == v.trim()).unwrap_or(0) as i32
                }
                "theme" => p.theme = THEMES.iter().position(|s| *s == v.trim()).unwrap_or(0) as i32,
                "format" => {
                    p.format = v
                        .parse::<i32>()
                        .ok()
                        .filter(|n| (0..=6).contains(n))
                        .unwrap_or(0)
                }
                "source_folder_hex" => p.source_folder = unhex(v),
                "output_folder_hex" => p.output_folder = unhex(v),
                "avif_quality" => {
                    p.avif_quality = v
                        .parse::<i32>()
                        .ok()
                        .filter(|n| (1..=100).contains(n))
                        .unwrap_or(90)
                }
                "webp_lossless" => p.webp_lossless = v != "false",
                "notify" => p.notify = v != "false",
                _ => {}
            }
        }
    }
    p
}
pub fn load(path: &Path) -> Preferences {
    std::fs::read_to_string(path)
        .map(|s| parse(&s))
        .unwrap_or_default()
}
pub fn save(path: &Path, p: Preferences) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    write!(
        f,
        "language={}\ntheme={}\nformat={}\nsource_folder_hex={}\noutput_folder_hex={}\navif_quality={}\nwebp_lossless={}\nnotify={}\n",
        LANGUAGES[p.language.clamp(0, 3) as usize],
        THEMES[p.theme.clamp(0, 2) as usize],
        p.format.clamp(0, 6),
        hex(&p.source_folder),
        hex(&p.output_folder),
        p.avif_quality.clamp(1, 100),
        p.webp_lossless,
        p.notify
    )?;
    f.as_file().sync_all()?;
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_roundtrip_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        assert_eq!(load(&path), Preferences::default());
        for language in 0..4 {
            for theme in 0..3 {
                let p = Preferences {
                    language,
                    theme,
                    format: 6,
                    source_folder: PathBuf::from("D:/來源 [1] = 圖"),
                    output_folder: PathBuf::from("E:/輸出"),
                    notify: false,
                    ..Preferences::default()
                };
                save(&path, p.clone()).unwrap();
                assert_eq!(load(&path), p);
            }
        }
        std::fs::write(
            &path,
            "language=unknown\ntheme=999\nformat=99\navif_quality=0\nsource_folder_hex=zz\n",
        )
        .unwrap();
        assert_eq!(load(&path), Preferences::default());
        let blocked = dir.path().join("folder");
        std::fs::create_dir(&blocked).unwrap();
        assert!(save(&blocked, Preferences::default()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
