//! `rawcat <file>...`: decodes raw recorder files and prints per-source record counts.
//!
//! Exits non-zero if any file cannot be read. A truncated file (process killed, disk full) is
//! reported with `truncated=true`, not as an error.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use raki_recorder::read_file;

fn main() -> ExitCode {
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if files.is_empty() {
        eprintln!("usage: rawcat <file.ndjson.zst>...");
        return ExitCode::from(2);
    }
    let mut failed = false;
    for file in &files {
        match read_file(file) {
            Ok(out) => {
                println!("{}", file.display());
                let mut by_source: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
                for r in &out.records {
                    let kind = serde_json::to_value(r.k)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    *by_source
                        .entry(r.s.as_str())
                        .or_default()
                        .entry(kind)
                        .or_default() += 1;
                }
                for (source, kinds) in &by_source {
                    let total: usize = kinds.values().sum();
                    let detail: Vec<String> =
                        kinds.iter().map(|(k, n)| format!("{k}={n}")).collect();
                    println!("  {source:<12} {total:>9}  ({})", detail.join(" "));
                }
                println!(
                    "  records={} bad_lines={} truncated={}",
                    out.records.len(),
                    out.bad_lines,
                    out.truncated
                );
            }
            Err(e) => {
                eprintln!("{}: {e}", file.display());
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
