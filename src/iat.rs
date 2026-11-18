//! 選用的外部驗證工具；不參與引擎驗證及輸出發布決策。
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const STREAM_LIMIT: u64 = 1024 * 1024;
const REPORT_LIMIT: u64 = 8 * 1024 * 1024;
static SLOT: Mutex<()> = Mutex::new(());
type ProcessOutput = Result<(i32, Vec<u8>, Vec<u8>), String>;

#[derive(Clone, Debug)]
pub struct Client {
    executable: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Different,
    Indeterminate,
    Invalid,
    NotChecked,
    ToolError,
    Unavailable,
    Cancelled,
}
impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Different => "different",
            Self::Indeterminate => "indeterminate",
            Self::Invalid => "invalid",
            Self::NotChecked => "not_checked",
            Self::ToolError => "tool_error",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Outcome {
    /// IMB-C 正規化判定碼；只有 Verified 可為 0。
    pub code: Option<i32>,
    /// 外部程序原始退出碼，與正規化判定分開保留。
    pub tool_exit_code: Option<i32>,
    pub verdict: Verdict,
    pub message: String,
    pub pixel_verified: Option<bool>,
}
impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "iat: {}", self.message)
    }
}
impl Outcome {
    pub fn unavailable(message: String) -> Self {
        Self {
            code: None,
            tool_exit_code: None,
            verdict: Verdict::Unavailable,
            message,
            pixel_verified: None,
        }
    }
    pub fn is_tool_failure(&self) -> bool {
        matches!(self.verdict, Verdict::ToolError | Verdict::Unavailable)
    }
    pub fn is_verified(&self) -> bool {
        self.verdict == Verdict::Verified
    }
    pub fn json(&self, lang: i32) -> Value {
        serde_json::json!({"code": self.code, "tool_exit_code": self.tool_exit_code,
            "verdict": self.verdict.as_str(), "verified": self.is_verified(), "status": match self.verdict {
                Verdict::Unavailable => "unavailable", Verdict::ToolError => "failed", Verdict::Cancelled => "cancelled",
                Verdict::NotChecked => "not_checked", _ => "completed" },
            "message": self.localized(lang), "avif_pixel_verified": self.pixel_verified})
    }
    pub fn localized(&self, lang: i32) -> String {
        let labels = [
            [
                "無損已實證",
                "无损已实证",
                "Lossless verified",
                "可逆性を実証",
            ],
            ["有差異", "有差异", "Differences found", "差分あり"],
            ["無法判定", "无法判定", "Indeterminate", "判定不能"],
            [
                "解碼或結構失敗",
                "解码或结构失败",
                "Decode or structure failure",
                "復号・構造エラー",
            ],
            ["未檢查", "未检查", "Not checked", "未検査"],
            ["工具錯誤", "工具错误", "Tool error", "ツールエラー"],
            [
                "未完成／不可用",
                "未完成／不可用",
                "Incomplete / unavailable",
                "未完了・利用不可",
            ],
        ];
        let index = match self.verdict {
            Verdict::Verified => 0,
            Verdict::Different => 1,
            Verdict::Indeterminate => 2,
            Verdict::Invalid => 3,
            Verdict::NotChecked => 4,
            Verdict::ToolError => 5,
            Verdict::Unavailable | Verdict::Cancelled => 6,
        };
        format!(
            "iat: {} ({})",
            labels[index][lang.clamp(0, 3) as usize],
            self.message
        )
    }
}

fn version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let mut parts = text.strip_prefix("iat ")?.split('.');
    let mut number = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let v = (number()?, number()?, number()?);
    if parts.next().is_some() {
        None
    } else {
        Some(v)
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err("報告或診斷資料超限".into());
    }
    let mut bytes = Vec::new();
    f.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("報告或診斷資料超限".into());
    }
    Ok(bytes)
}

#[cfg(test)]
thread_local! {
    static WATCH_TREE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OBSERVED_TREE: std::cell::RefCell<Vec<std::os::windows::io::OwnedHandle>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn validation_budget(limits: &BTreeMap<String, u64>) -> Result<u64, String> {
    let job = limits
        .get("job_commit_mib")
        .copied()
        .ok_or("缺少明確 Job 記憶體上限")?;
    let buffer = limits
        .get("buffer_mib")
        .copied()
        .ok_or("缺少明確緩衝區上限")?;
    if !(16..=4096).contains(&job) || !(1..=256).contains(&buffer) {
        return Err("外部驗證資源上限無效".into());
    }
    Ok(job + buffer + 128)
}
#[cfg(test)]
fn run(
    exe: &Path,
    args: &[std::ffi::OsString],
    dir: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> ProcessOutput {
    run_budgeted(exe, args, dir, timeout, cancel, 1216)
}
fn run_budgeted(
    exe: &Path,
    args: &[std::ffi::OsString],
    dir: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
    memory_mib: u64,
) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("未完成（已取消）".into());
    }
    let stdout = dir.join("stdout");
    let stderr = dir.join("stderr");
    let stdout_file = File::create(&stdout).map_err(|e| e.to_string())?;
    let stderr_file = File::create(&stderr).map_err(|e| e.to_string())?;
    let mut child =
        crate::engine::spawn_external_budgeted(exe, args, &stdout_file, &stderr_file, memory_mib)
            .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let result: Result<i32, String> = (|| {
        let code = loop {
            #[cfg(test)]
            if WATCH_TREE.with(|v| v.get()) {
                let handles = child.process_handles().map_err(|e| e.to_string())?;
                OBSERVED_TREE.with(|v| {
                    if handles.len() > v.borrow().len() {
                        *v.borrow_mut() = handles;
                    }
                });
            }
            if cancel.load(Ordering::Relaxed) {
                return Err("未完成（已取消）".into());
            }
            if start.elapsed() >= timeout {
                return Err("未完成（逾時）".into());
            }
            for path in [&stdout, &stderr, &dir.join("report.json")] {
                if let Ok(meta) = fs::metadata(path)
                    && meta.len()
                        > if path == &dir.join("report.json") {
                            REPORT_LIMIT
                        } else {
                            STREAM_LIMIT
                        }
                {
                    return Err("報告或診斷資料超限".into());
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status.code().ok_or("工具異常結束")?;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        Ok(code)
    })();
    // 正常結束也要確認整個 Job（包括任何殘留 worker）active=0，才釋放預留及暫存。
    child
        .stop()
        .map_err(|e| format!("無法確認外部程序樹停止：{e}"))?;
    let code = result?;
    Ok((
        code,
        read_bounded(&stdout, STREAM_LIMIT)?,
        read_bounded(&stderr, STREAM_LIMIT)?,
    ))
}

impl Client {
    /// 只找設定的絕對路徑，再找應用程式旁，不搜尋 PATH／工作目錄。
    pub fn discover(configured: Option<&Path>, application: &Path) -> Result<Self, String> {
        if !application.is_absolute() {
            return Err("應用程式路徑必須是絕對路徑".into());
        }
        let sibling = application.with_file_name("iat.exe");
        let executable = match configured.filter(|p| !p.as_os_str().is_empty()) {
            Some(p) if !p.is_absolute() => return Err("工具設定必須是絕對路徑".into()),
            Some(p) if p.is_file() => p.to_owned(),
            Some(_) => return Err("指定的外部工具路徑不存在".into()),
            _ if sibling.is_file() => sibling,
            _ => return Err("找不到選用的外部驗證工具".into()),
        };
        if !executable.with_file_name("iat-worker.exe").is_file() {
            return Err("缺少 iat-worker.exe".into());
        }
        let dir = crate::engine::ValidationTemp::new().map_err(|e| e.to_string())?;
        let memory = crate::engine::reserve_validation_memory(128, &AtomicBool::new(false))?;
        let probe = run_budgeted(
            &executable,
            &["--version".into()],
            dir.path(),
            Duration::from_secs(10),
            &AtomicBool::new(false),
            128,
        );
        if probe.as_ref().is_err_and(|e| e.starts_with("無法確認")) {
            dir.retain();
            std::mem::forget(memory);
        }
        let (code, stdout, _) = probe?;
        let v = version(std::str::from_utf8(&stdout).map_err(|e| e.to_string())?)
            .ok_or("版本格式無效")?;
        if code != 0 || v < (0, 5, 1) {
            return Err("需要 iat 0.5.1 以上".into());
        }
        Ok(Self { executable })
    }

    /// 呼叫端必須先關閉輸出的寫入控制代碼；結果不刪除、覆寫任何圖片。
    pub fn compare(&self, original: &Path, output: &Path, cancel: &AtomicBool) -> Outcome {
        match self.compare_inner(original, output, cancel) {
            Ok(result) => result,
            Err(message) => Outcome {
                code: None,
                tool_exit_code: None,
                verdict: if cancel.load(Ordering::Relaxed) {
                    Verdict::Cancelled
                } else {
                    Verdict::ToolError
                },
                message,
                pixel_verified: None,
            },
        }
    }

    fn compare_inner(
        &self,
        original: &Path,
        output: &Path,
        cancel: &AtomicBool,
    ) -> Result<Outcome, String> {
        self.compare_using(
            original,
            output,
            cancel,
            memory_ceiling_mib(),
            |args, dir, limits| {
                let budget = validation_budget(limits)?;
                let memory = crate::engine::reserve_validation_memory(budget, cancel)?;
                let result = run_budgeted(
                    &self.executable,
                    args,
                    dir,
                    Duration::from_secs(720),
                    cancel,
                    budget,
                );
                if result.as_ref().is_err_and(|e| e.starts_with("無法確認")) {
                    std::mem::forget(memory); // 未確認停止時保留跨程序預留至執行個體退出。
                }
                result
            },
        )
    }
    fn compare_using(
        &self,
        original: &Path,
        output: &Path,
        cancel: &AtomicBool,
        ceiling: u64,
        mut execute: impl FnMut(&[std::ffi::OsString], &Path, &BTreeMap<String, u64>) -> ProcessOutput,
    ) -> Result<Outcome, String> {
        let original = original
            .canonicalize()
            .map_err(|e| format!("來源路徑無法解析：{e}"))?;
        let output = output
            .canonicalize()
            .map_err(|e| format!("輸出路徑無法解析：{e}"))?;
        // 最多一個外部驗證，避免兩個引擎工作再各啟動高記憶體解碼器。
        let _slot = loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("未完成（已取消）".into());
            }
            match SLOT.try_lock() {
                Ok(slot) => break slot,
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(25)),
                Err(TryLockError::Poisoned(_)) => return Err("外部驗證排程不可用".into()),
            }
        };
        let mut limits = BTreeMap::from([
            ("job_commit_mib".to_owned(), 1024),
            ("buffer_mib".to_owned(), 64),
        ]);
        for attempt in 0..2 {
            let dir = crate::engine::ValidationTemp::new().map_err(|e| e.to_string())?;
            let report = dir.path().join("report.json");
            let mut args = vec![
                "compare".into(),
                original.as_os_str().to_owned(),
                output.as_os_str().to_owned(),
                "--report-json".into(),
                report.as_os_str().to_owned(),
                "--temp-dir".into(),
                dir.path().as_os_str().to_owned(),
            ];
            for (name, value) in &limits {
                args.extend(["--limits".into(), format!("{name}={value}").into()]);
            }
            if cancel.load(Ordering::Relaxed) {
                return Err("未完成（已取消）".into());
            }
            let execution = execute(&args, dir.path(), &limits);
            if execution.as_ref().is_err_and(|e| e.starts_with("無法確認")) {
                dir.retain();
            }
            let (code, _, stderr) = execution?;
            if code == 5 {
                return Ok(Outcome {
                    code: Some(5),
                    tool_exit_code: Some(5),
                    verdict: Verdict::ToolError,
                    message: format!("工具錯誤：{}", String::from_utf8_lossy(&stderr)),
                    pixel_verified: None,
                });
            }
            let bytes = read_bounded(&report, REPORT_LIMIT)?;
            let json: Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("驗證報告無法解析：{e}"))?;
            let result = parse_report(code, &json)?;
            if code == 4 && attempt == 0 {
                let retry = retry_limits(&json, ceiling);
                if !retry.is_empty() {
                    limits.extend(retry);
                    continue;
                }
            }
            return Ok(result);
        }
        Err("未檢查".into())
    }
}

fn parse_report(code: i32, json: &Value) -> Result<Outcome, String> {
    let schema = json["schema_version"].as_str().ok_or("報告缺少 Schema")?;
    let (major, minor) = schema.split_once('.').ok_or("Schema 格式無效")?;
    if major != "1"
        || !minor.bytes().all(|c| c.is_ascii_digit())
        || minor.parse::<u64>().ok().is_none_or(|n| n < 1)
    {
        return Err("不相容的報告 Schema".into());
    }
    if json["mode"] != "compare"
        || json["strict"]["result"].as_str().is_none()
        || json["strict"]["reasons"].as_array().is_none()
    {
        return Err("驗證報告缺少必要欄位".into());
    }
    let pixel_verified = json["output"]["avif"]["pixel_verified"].as_bool();
    if json["output"]["format"].as_str().is_none() {
        return Err("報告缺少輸出格式".into());
    }
    let is_avif = json["output"]["format"] == "avif";
    if code == 0 && json["strict"]["result"] != "match" {
        return Err("退出碼與嚴格比對結果不一致".into());
    }
    let normalized = if code == 0 && is_avif && pixel_verified != Some(true) {
        2
    } else {
        code
    };
    let verdict = match normalized {
        0 => Verdict::Verified,
        1 => Verdict::Different,
        2 => Verdict::Indeterminate,
        3 => Verdict::Invalid,
        4 => Verdict::NotChecked,
        5 => Verdict::ToolError,
        _ => return Err(format!("未知退出碼 {code}")),
    };
    let message = match code {
        0 if is_avif && pixel_verified != Some(true) => "無法判定（AVIF 尚無像素實證）",
        0 if json["strict"]["result"] != "match" => return Err("退出碼與嚴格比對結果不一致".into()),
        0 => "無損已實證",
        1 => "有差異",
        2 => "無法判定",
        3 => "解碼或結構失敗",
        4 => "未檢查",
        5 => "工具錯誤",
        _ => return Err(format!("未知退出碼 {code}")),
    };
    let reasons = json["strict"]["reasons"]
        .as_array()
        .ok_or("缺少原因")?
        .iter()
        .filter_map(Value::as_str)
        .take(64)
        .collect::<Vec<_>>()
        .join(", ");
    let issues = json["issues"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter(|i| !(i["code"] == "psd_composite_flag_absent" && i["scope"] == "output"))
                .filter_map(|i| i["code"].as_str())
                .take(64)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok(Outcome {
        code: Some(normalized),
        tool_exit_code: Some(code),
        verdict,
        message: format!("{message}；{reasons}；{issues}"),
        pixel_verified,
    })
}

fn retry_limits(json: &Value, memory_mib: u64) -> BTreeMap<String, u64> {
    let mut limits = BTreeMap::new();
    for side in ["original", "output"] {
        let need = &json[side]["resource_need"];
        if need.is_null() {
            continue;
        }
        let Some(name) = need["limit"].as_str() else {
            return BTreeMap::new();
        };
        let Some(value) = need["suggested_value"].as_u64() else {
            return BTreeMap::new();
        };
        let Some(current) = need["current_value"].as_u64() else {
            return BTreeMap::new();
        };
        let cap = match name {
            "job_commit_mib" => memory_mib.min(4096),
            "buffer_mib" => 256,
            _ => return BTreeMap::new(),
        };
        if value <= current
            || value > cap
            || need["suggested_argument"] != format!("--limits {name}={value}")
        {
            return BTreeMap::new();
        }
        let entry = limits.entry(name.to_owned()).or_insert(value);
        *entry = (*entry).max(value);
    }
    limits
}

fn memory_ceiling_mib() -> u64 {
    crate::engine::external_validation_memory_mib()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn avif_verdict_preserves_tool_code_without_false_verified() {
        let mut j = json!({"schema_version":"1.1","mode":"compare","strict":{"result":"match","reasons":[]},"output":{"format":"avif","avif":{"pixel_verified":true}},"issues":[]});
        assert!(parse_report(0, &j).unwrap().is_verified());
        for proof in [Value::Null, json!(false)] {
            j["output"]["avif"]["pixel_verified"] = proof;
            let o = parse_report(0, &j).unwrap();
            assert_eq!(o.code, Some(2));
            assert_eq!(o.tool_exit_code, Some(0));
            assert_eq!(o.verdict, Verdict::Indeterminate);
            assert!(!o.json(0)["verified"].as_bool().unwrap());
        }
        j["strict"]["result"] = json!("mismatch");
        assert_eq!(parse_report(1, &j).unwrap().verdict, Verdict::Different);
        assert_eq!(parse_report(3, &j).unwrap().verdict, Verdict::Invalid);
    }
    #[test]
    fn first_and_retry_limits_have_explicit_budget_and_outer_ceiling() {
        assert!(validation_budget(&BTreeMap::new()).is_err());
        assert_eq!(
            validation_budget(&BTreeMap::from([
                ("job_commit_mib".into(), 1024),
                ("buffer_mib".into(), 64)
            ]))
            .unwrap(),
            1216
        );
        assert_eq!(
            validation_budget(&BTreeMap::from([
                ("job_commit_mib".into(), 4096),
                ("buffer_mib".into(), 256)
            ]))
            .unwrap(),
            4480
        );
        assert!(
            validation_budget(&BTreeMap::from([
                ("job_commit_mib".into(), 4097),
                ("buffer_mib".into(), 64)
            ]))
            .is_err()
        );
    }
    #[test]
    fn retry_workflow_uses_new_reports_once_and_preserves_both_inputs() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        let output = root.path().join("output");
        fs::write(&original, b"original").unwrap();
        fs::write(&output, b"completed output").unwrap();
        let client = Client {
            executable: root.path().join("not-executed.exe"),
        };
        for terminal in [0, 4] {
            let mut calls = 0;
            let mut reports = Vec::new();
            let outcome=client.compare_using(&original,&output,&AtomicBool::new(false),4096,|args,dir,limits|{
                calls+=1; assert!(args.iter().any(|a|a=="--limits")); if calls==1 { assert_eq!(limits.get("job_commit_mib"),Some(&1024)); assert_eq!(limits.get("buffer_mib"),Some(&64)); } let report=dir.join("report.json");assert!(!report.exists());reports.push(report.clone());
                if calls==2 {
                    assert_eq!(limits.get("job_commit_mib"),Some(&2048));assert_eq!(limits.get("buffer_mib"),Some(&128));
                    assert!(args.iter().any(|a|a=="job_commit_mib=2048"));assert!(args.iter().any(|a|a=="buffer_mib=128"));
                }
                let code=if calls==1 {4} else {terminal};
                let mut j=json!({"schema_version":"1.1","mode":"compare","strict":{"result":if code==0 {"match"}else{"not_evaluated"},"reasons":[]},"output":{"format":"png"},"issues":[]});
                j["original"]["resource_need"]=json!({"limit":"job_commit_mib","current_value":1024,"suggested_value":2048,"suggested_argument":"--limits job_commit_mib=2048"});
                j["output"]["resource_need"]=json!({"limit":"buffer_mib","current_value":64,"suggested_value":128,"suggested_argument":"--limits buffer_mib=128"});
                fs::write(report,serde_json::to_vec(&j).unwrap()).unwrap();Ok((code,Vec::new(),Vec::new()))
            }).unwrap();
            assert_eq!(calls, 2);
            assert_ne!(reports[0], reports[1]);
            assert!(reports.iter().all(|p| !p.exists()));
            assert_eq!(outcome.code, Some(terminal));
        }
        let mut calls = 0;
        let result = client
            .compare_using(
                &original,
                &output,
                &AtomicBool::new(false),
                4096,
                |_, _, _| {
                    calls += 1;
                    Ok((5, Vec::new(), b"report failed".to_vec()))
                },
            )
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result.code, Some(5));
        assert_eq!(fs::read(original).unwrap(), b"original");
        assert_eq!(fs::read(output).unwrap(), b"completed output");
    }
    #[test]
    fn subprocess_timeout_and_cancel_are_bounded() {
        let exe = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let args = vec![
            "-NoProfile".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 5".into(),
        ];
        let d = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let e = run(
            &exe,
            &args,
            d.path(),
            Duration::from_millis(100),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(e.contains("逾時"));
        assert!(start.elapsed() < Duration::from_secs(3));
        let c = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = c.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c2.store(true, Ordering::Relaxed);
        });
        assert!(
            run(&exe, &args, d.path(), Duration::from_secs(5), &c)
                .unwrap_err()
                .contains("取消")
        );
        t.join().unwrap();
    }
    #[test]
    fn strict_versions() {
        assert_eq!(version("iat 0.5.1\r\n"), Some((0, 5, 1)));
        for s in [
            "iat 0.5.1 extra",
            "iat 0.5.1\nother",
            "iat +0.5.1",
            "iat 0.5",
        ] {
            assert_eq!(version(s), None);
        }
    }
    #[test]
    fn process_codes_schema_and_avif_proof() {
        let mut j = json!({"schema_version":"1.1", "mode":"compare", "strict":{"result":"match", "reasons":[]}, "output":{"format":"avif", "avif":{"pixel_verified":true}}, "issues":[]});
        for code in 0..=5 {
            assert_eq!(parse_report(code, &j).unwrap().code, Some(code));
        }
        j["output"]["avif"]["pixel_verified"] = Value::Null;
        assert!(
            parse_report(0, &j)
                .unwrap()
                .message
                .contains("尚無像素實證")
        );
        j["schema_version"] = json!("2.1");
        assert!(parse_report(0, &j).is_err());
        j["schema_version"] = json!("1.0");
        assert!(parse_report(0, &j).is_err());
        assert!(parse_report(7, &j).is_err());
    }
    #[test]
    fn retry_is_bounded_deduplicated_and_never_executes_report_text() {
        let need = json!({"limit":"job_commit_mib","current_value":1024,"suggested_value":2048,"suggested_argument":"--limits job_commit_mib=2048"});
        let mut j = json!({"original":{"resource_need":need},"output":{"resource_need":need}});
        assert_eq!(retry_limits(&j, 4096).len(), 1);
        assert!(retry_limits(&j, 1024).is_empty());
        j["output"]["resource_need"]["suggested_argument"] =
            json!("--limits job_commit_mib=2048 --dangerous");
        assert!(retry_limits(&j, 4096).is_empty());
        j["output"]["resource_need"]["suggested_value"] = Value::Null;
        assert!(retry_limits(&j, 4096).is_empty());
    }
    #[test]
    #[ignore = "Subprocess fixture for actual_iat_owner_exit_stops_nested_worker"]
    fn actual_iat_owner_exit_fixture() {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetProcessId(handle: *mut std::ffi::c_void) -> u32;
        }
        let Some(marker) = std::env::var_os("IMBC_IAT_EXIT_FIXTURE") else {
            return;
        };
        let marker = PathBuf::from(marker);
        let exe = PathBuf::from(std::env::var_os("IMBC_TEST_IAT").unwrap());
        let source = PathBuf::from(std::env::var_os("IMBC_TEST_IAT_SOURCE").unwrap());
        let dir = crate::engine::ValidationTemp::new().unwrap();
        let stdout = File::create(dir.path().join("stdout")).unwrap();
        let stderr = File::create(dir.path().join("stderr")).unwrap();
        let args = vec![
            "compare".into(),
            source.as_os_str().to_owned(),
            source.as_os_str().to_owned(),
            "--report-json".into(),
            dir.path().join("report.json").into_os_string(),
        ];
        let child = crate::engine::spawn_external_command(&exe, &args, &stdout, &stderr).unwrap();
        let start = Instant::now();
        loop {
            let handles = child.process_handles().unwrap();
            if handles.len() >= 2 {
                let pids: Vec<u32> = handles
                    .iter()
                    .map(|h| unsafe { GetProcessId(h.as_raw_handle()) })
                    .collect();
                fs::write(
                    marker.join("ready.json"),
                    serde_json::to_vec(&json!({"pids": pids, "temp":dir.path().to_string_lossy()}))
                        .unwrap(),
                )
                .unwrap();
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
        }
        // 呼叫端以 TerminateProcess 結束本測試程序，刻意不執行解構器。
        std::thread::sleep(Duration::from_secs(10));
    }
    #[test]
    #[ignore = "Actual private iat plus abrupt owner termination; explicit local integration only"]
    fn actual_iat_owner_exit_stops_nested_worker() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, ms: u32) -> u32;
        }
        let marker = tempfile::tempdir().unwrap();
        let mut owner = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "iat::tests::actual_iat_owner_exit_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("IMBC_IAT_EXIT_FIXTURE", marker.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let ready = marker.path().join("ready.json");
        let start = Instant::now();
        while !ready.is_file() {
            if start.elapsed() > Duration::from_secs(10) {
                let _ = owner.kill();
                panic!("worker fixture did not start");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let j: Value = serde_json::from_slice(&fs::read(&ready).unwrap()).unwrap();
        let handles: Vec<_> = j["pids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let h = unsafe { OpenProcess(0x100000, 0, v.as_u64().unwrap() as u32) };
                assert!(!h.is_null());
                let h = unsafe { OwnedHandle::from_raw_handle(h) };
                assert_eq!(unsafe { WaitForSingleObject(h.as_raw_handle(), 0) }, 258);
                h
            })
            .collect();
        owner.kill().unwrap();
        owner.wait().unwrap();
        for h in handles {
            assert_eq!(unsafe { WaitForSingleObject(h.as_raw_handle(), 5000) }, 0);
        }
        let residue = PathBuf::from(j["temp"].as_str().unwrap());
        assert!(residue.is_dir());
        let _ = crate::engine::recover_stale_temp();
        assert!(!residue.exists());
    }
    #[test]
    #[ignore = "Actual private iat; set IMBC_TEST_IAT and IMBC_TEST_IAT_SOURCE"]
    fn actual_iat_nested_worker_timeout_cancel_and_completion() {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, ms: u32) -> u32;
        }
        let exe = PathBuf::from(std::env::var_os("IMBC_TEST_IAT").unwrap());
        let source = PathBuf::from(std::env::var_os("IMBC_TEST_IAT_SOURCE").unwrap());
        let client = Client::discover(Some(&exe), &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(client.executable, exe);
        for mode in ["timeout", "cancel", "complete"] {
            let d = crate::engine::ValidationTemp::new().unwrap();
            let args = vec![
                "compare".into(),
                source.as_os_str().to_owned(),
                source.as_os_str().to_owned(),
                "--report-json".into(),
                d.path().join("report.json").into_os_string(),
                "--temp-dir".into(),
                d.path().as_os_str().to_owned(),
            ];
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            let c = cancel.clone();
            let thread = (mode == "cancel").then(|| {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(250));
                    c.store(true, Ordering::Relaxed);
                })
            });
            WATCH_TREE.with(|v| v.set(true));
            OBSERVED_TREE.with(|v| v.borrow_mut().clear());
            let result = run(
                &exe,
                &args,
                d.path(),
                if mode == "timeout" {
                    Duration::from_millis(250)
                } else {
                    Duration::from_secs(90)
                },
                &cancel,
            );
            WATCH_TREE.with(|v| v.set(false));
            if let Some(t) = thread {
                t.join().unwrap();
            }
            if mode == "complete" {
                assert!(matches!(result.unwrap().0, 0 | 2));
            } else {
                assert!(result.unwrap_err().contains(if mode == "timeout" {
                    "逾時"
                } else {
                    "取消"
                }));
            }
            OBSERVED_TREE.with(|v| {
                let handles = v.borrow();
                assert!(
                    handles.len() >= 2,
                    "actual iat-worker was not observed: {mode}"
                );
                for handle in handles.iter() {
                    assert_eq!(
                        unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                        0
                    );
                }
            });
            println!("PASS actual iat nested Job: {mode}; parent + worker exited");
        }
    }
    #[test]
    fn configured_missing_path_never_falls_back_to_sibling() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("iat.exe"), b"not executable").unwrap();
        fs::write(d.path().join("iat-worker.exe"), b"not executable").unwrap();
        let error = Client::discover(
            Some(&d.path().join("missing.exe")),
            &d.path().join("app.exe"),
        )
        .unwrap_err();
        assert!(error.contains("指定"), "{error}");
    }
    #[test]
    fn spawn_failure_and_abnormal_exit_are_reported() {
        let d = tempfile::tempdir().unwrap();
        assert!(
            run(
                &d.path().join("missing.exe"),
                &[],
                d.path(),
                Duration::from_secs(2),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let exe = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        assert_eq!(
            run(
                &exe,
                &["-NoProfile".into(), "-Command".into(), "exit 7".into()],
                d.path(),
                Duration::from_secs(10),
                &AtomicBool::new(false)
            )
            .unwrap()
            .0,
            7
        );
    }
    #[test]
    fn missing_tool_and_relative_path_are_optional_errors() {
        let d = tempfile::tempdir().unwrap();
        assert!(Client::discover(None, &d.path().join("app.exe")).is_err());
        assert!(Client::discover(Some(Path::new("iat.exe")), &d.path().join("app.exe")).is_err());
    }
}
