//! Complete warning journal on disk; UI reads fixed-size pages, never the whole batch.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub const PAGE_BYTES: usize = 32 * 1024;
pub fn clean_successes(total: usize, failed: usize, cancelled: usize, warned: usize) -> usize {
    total.saturating_sub(failed.saturating_add(cancelled).saturating_add(warned))
}
#[derive(Default, Clone, Debug)]
pub struct Summary {
    pub files: usize,
    pub messages: usize,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}
pub struct Journal {
    root: PathBuf,
    file: Option<File>,
    summary: Summary,
}
impl Journal {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            file: None,
            summary: Summary::default(),
        }
    }
    pub fn record(&mut self, input: &Path, warnings: &[String]) -> io::Result<()> {
        if warnings.is_empty() {
            return Ok(());
        }
        self.summary.files += 1;
        self.summary.messages += warnings.len();
        let result = self.write_record(input, warnings);
        if let Err(e) = &result {
            // No unlimited in-memory fallback. Stop the batch and expose storage failure,
            // with a bounded excerpt in the persistent completion status/viewer.
            if self.summary.error.is_none() {
                let detail = format!(
                    "警告報告無法完整保存：{e}；來源 {input:?}；本次警告：{}",
                    warnings.join("；")
                );
                self.summary.error = Some(detail.chars().take(8192).collect());
            }
        }
        result
    }
    fn write_record(&mut self, input: &Path, warnings: &[String]) -> io::Result<()> {
        if self.summary.error.is_some() {
            return Err(io::Error::other("warning report is incomplete"));
        }
        if self.file.is_none() {
            let temp = tempfile::Builder::new()
                .rand_bytes(16)
                .prefix("IMB-C-warnings-")
                .suffix(".txt")
                .tempfile_in(&self.root)?;
            let (file, path) = temp.keep().map_err(|e| e.error)?;
            self.file = Some(file);
            self.summary.path = Some(path);
            writeln!(
                self.file.as_mut().unwrap(),
                "IMB-C warning report (UTF-8)\n"
            )?;
        }
        let f = self.file.as_mut().unwrap();
        writeln!(f, "[{}] Source: {input:?}", self.summary.files)?;
        for warning in warnings {
            writeln!(f, "Warning: {warning}")?;
        }
        writeln!(f)?;
        f.flush()
    }
    pub fn finish(&mut self, completed: usize, failed: usize, cancelled: usize) -> Summary {
        if let Some(file) = &mut self.file {
            let result=writeln!(file,"Completed: {completed}; with warnings: {}; warning messages: {}; failed: {failed}; cancelled: {cancelled}",self.summary.files,self.summary.messages).and_then(|_|file.sync_all());
            if let Err(e) = result {
                self.summary
                    .error
                    .get_or_insert_with(|| format!("警告報告儲存失敗，可能不完整：{e}"));
            }
        }
        self.file.take();
        self.summary.clone()
    }
}
pub struct Page {
    pub text: String,
    pub previous: bool,
    pub next: bool,
}
pub fn read_page(path: &Path, index: u64) -> io::Result<Page> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = index
        .checked_mul(PAGE_BYTES as u64)
        .ok_or_else(|| io::Error::other("warning page overflow"))?;
    if start > size {
        return Err(io::Error::other("warning page out of range"));
    }
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(PAGE_BYTES + 3);
    file.take((PAGE_BYTES + 3) as u64).read_to_end(&mut bytes)?;
    // Include a UTF-8 character crossing the end; next page skips its continuation.
    let mut first = 0;
    while first < bytes.len() && bytes[first] & 0xc0 == 0x80 {
        first += 1;
    }
    let mut end = bytes.len().min(PAGE_BYTES);
    while end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
        end += 1;
    }
    let text = std::str::from_utf8(&bytes[first..end])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        .to_owned();
    Ok(Page {
        text,
        previous: index > 0,
        next: start.saturating_add(PAGE_BYTES as u64) < size,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r3_warning_journal_keeps_every_record_without_memory_growth() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::new(dir.path());
        for i in 0..350 {
            j.record(
                Path::new(&format!("{i:03}.avif")),
                &[format!(
                    "important warning {i:03} 中文色彩未驗證；請保留來源與檢查完整警告報告"
                )],
            )
            .unwrap();
        }
        let summary = j.finish(400, 3, 2);
        assert_eq!(summary.files, 350);
        assert_eq!(summary.messages, 350);
        assert!(summary.error.is_none());
        let mut page = 0;
        let mut all = String::new();
        loop {
            let p = read_page(summary.path.as_ref().unwrap(), page).unwrap();
            assert!(p.text.len() <= PAGE_BYTES + 3);
            all.push_str(&p.text);
            if !p.next {
                break;
            }
            page += 1;
        }
        assert!(page > 0);
        assert!(all.contains("important warning 000") && all.contains("important warning 349"));
        assert_eq!(all.matches("Source:").count(), 350);
    }
    #[test]
    fn r3_warning_journal_storage_failure_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::new(&dir.path().join("missing"));
        assert!(
            j.record(
                Path::new("original.avif"),
                &["important missing color".into()]
            )
            .is_err()
        );
        let s = j.finish(1, 0, 0);
        assert_eq!(s.files, 1);
        assert_eq!(s.messages, 1);
        assert!(s.error.unwrap().contains("important missing color"));
        assert!(s.path.is_none());
    }
    #[test]
    fn r3_no_warning_batch_creates_no_report() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::new(dir.path());
        j.record(Path::new("ok.png"), &[]).unwrap();
        let s = j.finish(1, 0, 0);
        assert_eq!(s.files, 0);
        assert!(s.path.is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn r3_warning_pages_keep_utf8_at_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.txt");
        let text = format!(
            "{}{}{}",
            "x".repeat(PAGE_BYTES - 1),
            "色彩警告",
            "y".repeat(PAGE_BYTES)
        );
        std::fs::write(&path, &text).unwrap();
        let mut all = String::new();
        for i in 0..3 {
            all.push_str(&read_page(&path, i).unwrap().text);
        }
        assert_eq!(all, text);
    }
}
