//! 僅供 CLI 契約測試的產生式外部程序；不解析圖片、不啟動 worker。
use std::{env, fs, process};
fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let mode = env::var("IMBC_STUB_MODE").unwrap_or_else(|_| "verified".into());
    if args.first().is_some_and(|a| a == "--version") {
        println!("iat {}", if mode == "old" { "0.4.1" } else { "0.5.1" });
        return;
    }
    if !args.iter().any(|a| a == "job_commit_mib=1024")
        || !args.iter().any(|a| a == "buffer_mib=64")
    {
        eprintln!("missing explicit resource limits");
        process::exit(5);
    }
    if mode == "tool_error" {
        eprintln!("fixture tool failure");
        process::exit(5);
    }
    let report = args
        .iter()
        .position(|a| a == "--report-json")
        .map(|n| &args[n + 1])
        .unwrap();
    let (code, result, proof) = match mode.as_str() {
        "different" => (1, "mismatch", "false"),
        "invalid" => (3, "indeterminate", "null"),
        "indeterminate" => (0, "match", "null"),
        _ => (0, "match", "true"),
    };
    let schema = if mode == "bad_schema" { "2.0" } else { "1.1" };
    let json = format!(
        r#"{{"schema_version":"{schema}","mode":"compare","strict":{{"result":"{result}","reasons":[]}},"output":{{"format":"avif","avif":{{"pixel_verified":{proof}}}}},"issues":[]}}"#
    );
    fs::write(report, json).unwrap();
    process::exit(code);
}
