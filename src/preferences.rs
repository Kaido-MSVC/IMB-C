use std::{
    fs::OpenOptions,
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
    pub avif_lossless: bool,
    pub jxl_quality: i32,
    pub jxl_lossless: bool,
    pub iat_path: PathBuf,
    pub iat_enabled: bool,
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
            avif_lossless: false,
            jxl_quality: 90,
            jxl_lossless: true,
            iat_path: PathBuf::new(),
            iat_enabled: false,
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
    use std::os::windows::ffi::OsStrExt;
    format!(
        "w16:{}",
        path.as_os_str()
            .encode_wide()
            .map(|c| format!("{c:04x}"))
            .collect::<String>()
    )
}
fn unhex(s: &str) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    if let Some(s) = s.strip_prefix("w16:") {
        let units: Option<Vec<u16>> = s
            .as_bytes()
            .chunks(4)
            .map(|c| {
                if c.len() == 4 {
                    std::str::from_utf8(c)
                        .ok()
                        .and_then(|s| u16::from_str_radix(s, 16).ok())
                } else {
                    None
                }
            })
            .collect();
        return units
            .map(|v| std::ffi::OsString::from_wide(&v).into())
            .unwrap_or_default();
    }
    let bytes: Option<Vec<u8>> = s
        .as_bytes()
        .chunks(2)
        .map(|c| {
            if c.len() == 2 {
                std::str::from_utf8(c)
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
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
                        .filter(|n| (0..=7).contains(n))
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
                "avif_lossless" => p.avif_lossless = v == "true",
                "jxl_lossless" => p.jxl_lossless = v != "false",
                "jxl_quality" => {
                    p.jxl_quality = v
                        .parse::<i32>()
                        .ok()
                        .filter(|n| (1..=99).contains(n))
                        .unwrap_or(90)
                }
                "iat_path_hex" => p.iat_path = unhex(v),
                "iat_enabled" => p.iat_enabled = v == "true",
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
fn entries(p: &Preferences) -> [(&'static str, String); 13] {
    [
        (
            "language",
            LANGUAGES[p.language.clamp(0, 3) as usize].into(),
        ),
        ("theme", THEMES[p.theme.clamp(0, 2) as usize].into()),
        ("format", p.format.clamp(0, 7).to_string()),
        ("source_folder_hex", hex(&p.source_folder)),
        ("output_folder_hex", hex(&p.output_folder)),
        ("avif_quality", p.avif_quality.clamp(1, 100).to_string()),
        ("avif_lossless", p.avif_lossless.to_string()),
        ("jxl_quality", p.jxl_quality.clamp(1, 99).to_string()),
        ("jxl_lossless", p.jxl_lossless.to_string()),
        ("iat_path_hex", hex(&p.iat_path)),
        ("iat_enabled", p.iat_enabled.to_string()),
        ("webp_lossless", p.webp_lossless.to_string()),
        ("notify", p.notify.to_string()),
    ]
}

/// 原子寫入本次變更；保留其他欄位、註解，以及未識別的設定。
pub fn save_changes(path: &Path, old: &Preferences, new: &Preferences) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing directory"))?;
    std::fs::create_dir_all(parent)?;
    // 暫存檔替換會更換偏好檔的檔案控制代碼，使用固定的旁檔保護讀取、合併與替換。
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("ini.lock"))?;
    lock.try_lock()?;
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let changes: Vec<_> = entries(new)
        .into_iter()
        .zip(entries(old))
        .filter_map(|(new, old)| (existing.is_none() || new != old).then_some(new))
        .collect();
    if changes.is_empty() {
        return Ok(());
    }
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(existing) = existing {
        for line in existing.split_inclusive('\n') {
            if line
                .split_once('=')
                .is_some_and(|(key, _)| changes.iter().any(|(changed, _)| key.trim() == *changed))
            {
                continue;
            }
            f.write_all(line.as_bytes())?;
            if !line.ends_with('\n') {
                writeln!(f)?;
            }
        }
    }
    for (key, value) in changes {
        writeln!(f, "{key}={value}")?;
    }
    f.as_file().sync_all()?;
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_path_encoding_preserves_surrogates_and_reads_legacy_hex() {
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            68, 58, 92, 0xd800, 46, 101, 120, 101,
        ]));
        assert_eq!(unhex(&hex(&path)), path);
        assert_eq!(
            unhex("443a2f746f6f6c732f6961742e657865"),
            PathBuf::from("D:/tools/iat.exe")
        );
        assert!(unhex("w16:000").as_os_str().is_empty());
    }
    #[test]
    fn iat_opt_in_preserves_explicit_saved_preference() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("config.ini");
        assert!(!load(&path).iat_enabled);
        for (value, expected) in [("true", true), ("false", false), ("invalid", false)] {
            std::fs::write(&path, format!("iat_enabled={value}\n")).unwrap();
            assert_eq!(load(&path).iat_enabled, expected);
        }
        std::fs::write(&path, "language=2\n").unwrap();
        assert!(!load(&path).iat_enabled);
    }
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
                    format: 7,
                    avif_lossless: true,
                    jxl_lossless: false,
                    jxl_quality: 75,
                    iat_enabled: false,
                    iat_path: PathBuf::from("D:/tools/iat.exe"),
                    source_folder: PathBuf::from("D:/來源 [1] = 圖"),
                    output_folder: PathBuf::from("E:/輸出"),
                    notify: false,
                    ..Preferences::default()
                };
                let old = load(&path);
                save_changes(&path, &old, &p).unwrap();
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
        assert!(save_changes(&blocked, &Preferences::default(), &Preferences::default()).is_err());
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["config.ini", "config.ini.lock", "folder", "folder.ini.lock"]
        );
    }

    #[test]
    fn changing_appearance_preserves_legacy_values_and_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.ini");
        let old_text = "# user comment\r\noutput_folder_hex=443a2f756e617661696c61626c65\r\nnotify=false\r\nfuture_key=preserve=this";
        std::fs::write(&path, old_text).unwrap();
        let old = load(&path);
        let revised = Preferences {
            theme: 2,
            ..old.clone()
        };
        save_changes(&path, &old, &revised).unwrap();
        assert_eq!(load(&path), revised);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with(old_text)
        );

        // 使用者修復讀取問題前，必須保留原始位元組。
        std::fs::write(&path, [0xff, 0xfe, 0x30]).unwrap();
        assert!(save_changes(&path, &old, &revised).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe, 0x30]);
    }

    #[test]
    fn a_contended_save_preserves_both_changes_after_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.ini");
        let initial = "# keep\r\nunknown=preserve\r\n";
        std::fs::write(&path, initial).unwrap();
        let saved = load(&path);
        let theme_change = Preferences {
            theme: 2,
            ..saved.clone()
        };
        let language_change = Preferences {
            language: 3,
            ..saved.clone()
        };
        let first_writer = std::fs::File::create(path.with_extension("ini.lock")).unwrap();
        first_writer.try_lock().unwrap();
        assert_eq!(
            save_changes(&path, &saved, &language_change)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), initial);
        drop(first_writer);
        save_changes(&path, &saved, &theme_change).unwrap();
        save_changes(&path, &saved, &language_change).unwrap();
        let updated = load(&path);
        assert_eq!((updated.theme, updated.language), (2, 3));
        assert!(std::fs::read_to_string(path).unwrap().starts_with(initial));
    }
}
