use std::{
    fs,
    path::{Path, PathBuf},
};

// Take one snapshot before conversion. Never follow directory links/junctions.
pub fn folder(root: &Path, recursive: bool) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir != root => {
                warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        for entry in entries {
            let result = (|| -> std::io::Result<()> {
                let entry = entry?;
                let meta = entry.metadata()?;
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Ok(());
                    }
                }
                if entry.file_type()?.is_symlink() {
                    return Ok(());
                }
                let path = entry.path();
                if meta.is_dir() && recursive {
                    pending.push(path);
                } else if meta.is_file() && crate::engine::Format::input(&path).is_ok() {
                    files.push(path);
                }
                Ok(())
            })();
            if let Err(e) = result {
                warnings.push(format!("{}: {e}", dir.display()));
            }
        }
    }
    files.sort();
    files.dedup();
    Ok((files, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_scope_and_filter() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        for name in ["a.PNG", "b.tif", "sub/c.jxl", "skip.txt"] {
            fs::write(d.path().join(name), b"fixture").unwrap();
        }
        assert_eq!(folder(d.path(), false).unwrap().0.len(), 2);
        assert_eq!(folder(d.path(), true).unwrap().0.len(), 3);
        assert!(folder(&d.path().join("missing"), true).is_err());
    }
}
