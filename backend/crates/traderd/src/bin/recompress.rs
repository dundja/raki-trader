//! `recompress [--watch] <raw dir>`: recompresses finished raw recorder files at a high zstd level,
//! losslessly. Run it at low priority (`nice`): the recorder writes fast level-3 files, and this
//! shrinks them by about 30% once they are done.
//!
//! - A file is finished once it has not changed for `--settle-secs` (the recorder rotates hourly).
//! - The new file's decoded bytes must hash identically to the old one's before it replaces the old
//!   file (write to `.tmp`, fsync, rename). A file that does not decode to the end (truncated by a
//!   crash) is left as it is.
//! - `<raw dir>/.recompressed` lists every file handled, so each is done once.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail};
use clap::Parser;
use sha2::{Digest, Sha256};

const MANIFEST: &str = ".recompressed";

#[derive(Parser)]
#[command(about = "Recompresses finished raw recorder files at a high zstd level, losslessly")]
struct Args {
    /// The raw data directory (`data/raw`).
    dir: PathBuf,
    /// Keep running: look for finished files every `--every-secs`.
    #[arg(long)]
    watch: bool,
    #[arg(long, default_value_t = 19)]
    level: i32,
    /// A file untouched this long is finished.
    #[arg(long, default_value_t = 600)]
    settle_secs: u64,
    #[arg(long, default_value_t = 300)]
    every_secs: u64,
}

enum Done {
    Replaced { before: u64, after: u64 },
    NoGain { size: u64 },
    Truncated,
}

fn main() -> ExitCode {
    let args = Args::parse();
    loop {
        if let Err(e) = pass(&args) {
            eprintln!("recompress: {e:#}");
            if !args.watch {
                return ExitCode::FAILURE;
            }
        }
        if !args.watch {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(Duration::from_secs(args.every_secs));
    }
}

fn pass(args: &Args) -> anyhow::Result<()> {
    if !args.dir.is_dir() {
        return Ok(());
    }
    let manifest_path = args.dir.join(MANIFEST);
    let done = read_manifest(&manifest_path)?;
    let settle = Duration::from_secs(args.settle_secs);
    let mut files = vec![];
    collect(&args.dir, &mut files)?;
    files.sort();
    for path in files {
        let rel = path
            .strip_prefix(&args.dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        if done.contains(&rel) || !settled(&path, settle)? {
            continue;
        }
        let status = match recompress(&path, args.level)? {
            Done::Replaced { before, after } => {
                println!(
                    "recompress: {rel} {} MB -> {} MB",
                    before / 1_000_000,
                    after / 1_000_000
                );
                "replaced"
            }
            Done::NoGain { size } => {
                println!("recompress: {rel} kept ({} MB, no gain)", size / 1_000_000);
                "kept"
            }
            Done::Truncated => {
                println!("recompress: {rel} kept (truncated: does not decode to the end)");
                "truncated"
            }
        };
        let mut m = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&manifest_path)
            .context("open manifest")?;
        writeln!(m, "{rel}\t{status}")?;
        m.sync_all()?;
    }
    Ok(())
}

fn read_manifest(path: &Path) -> anyhow::Result<HashSet<String>> {
    match File::open(path) {
        Ok(f) => BufReader::new(f)
            .lines()
            .map(|l| Ok(l?.split('\t').next().unwrap_or_default().to_owned()))
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => Err(e).context("read manifest"),
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("list {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.to_string_lossy().ends_with(".ndjson.zst") {
            out.push(path);
        }
    }
    Ok(())
}

fn settled(path: &Path, settle: Duration) -> anyhow::Result<bool> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= settle))
}

fn recompress(path: &Path, level: i32) -> anyhow::Result<Done> {
    let before = fs::metadata(path)?.len();
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let result = write_recompressed(path, &tmp, level);
    let (original, written) = match result {
        Ok(Some(hashes)) => hashes,
        Ok(None) => {
            let _ = fs::remove_file(&tmp);
            return Ok(Done::Truncated);
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let check = hash_decoded(&tmp);
    if !matches!(check, Ok(Some(h)) if h == written && h == original) {
        let _ = fs::remove_file(&tmp);
        bail!("{}: recompressed file does not decode to the same bytes", path.display());
    }
    let after = fs::metadata(&tmp)?.len();
    if after >= before {
        fs::remove_file(&tmp)?;
        return Ok(Done::NoGain { size: before });
    }
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(Done::Replaced { before, after })
}

/// Decodes `src` into a new `level` file at `dst`. Returns the hash of the decoded bytes twice
/// (read side and written side), or `None` if `src` does not decode to the end.
fn write_recompressed(
    src: &Path,
    dst: &Path,
    level: i32,
) -> anyhow::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut decoder = zstd::stream::read::Decoder::new(File::open(src)?)?;
    let mut encoder = zstd::stream::write::Encoder::new(File::create(dst)?, level)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = match decoder.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Ok(None),
        };
        hasher.update(&buf[..n]);
        encoder.write_all(&buf[..n])?;
    }
    encoder.finish()?.sync_all()?;
    let h = hasher.finalize().to_vec();
    Ok(Some((h.clone(), h)))
}

/// The hash of `path`'s decoded bytes, or `None` if it does not decode to the end.
fn hash_decoded(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let mut decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match decoder.read(&mut buf) {
            Ok(0) => return Ok(Some(hasher.finalize().to_vec())),
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zst(path: &Path, body: &[u8], level: i32) {
        let mut e = zstd::stream::write::Encoder::new(File::create(path).unwrap(), level).unwrap();
        e.write_all(body).unwrap();
        e.finish().unwrap();
    }

    /// Book-like lines: repetitive structure, pseudo-random prices and sizes.
    fn body() -> Vec<u8> {
        let mut x: u64 = 7;
        (0..20_000)
            .map(|i| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                format!(
                    "{{\"t\":{i},\"s\":\"clob_market\",\"d\":\"{{\\\"price\\\":\\\"0.{:02}\\\",\\\"size\\\":\\\"{}.{:02}\\\"}}\"}}\n",
                    (x >> 33) % 100,
                    (x >> 40) % 5000,
                    (x >> 20) % 100
                )
            })
            .collect::<String>()
            .into_bytes()
    }

    fn decoded(path: &Path) -> Vec<u8> {
        let mut out = vec![];
        zstd::stream::read::Decoder::new(File::open(path).unwrap())
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn replaces_with_identical_content_and_records_it_once() {
        let tmp = tempfile::tempdir().unwrap();
        let day = tmp.path().join("2026-10-31");
        fs::create_dir_all(&day).unwrap();
        let file = day.join("10-s-0.ndjson.zst");
        zst(&file, &body(), 1);
        let before = fs::metadata(&file).unwrap().len();
        let args = Args {
            dir: tmp.path().to_path_buf(),
            watch: false,
            level: 19,
            settle_secs: 0,
            every_secs: 0,
        };
        pass(&args).unwrap();
        assert_eq!(decoded(&file), body(), "lossless");
        assert!(fs::metadata(&file).unwrap().len() < before);
        let manifest = fs::read_to_string(tmp.path().join(MANIFEST)).unwrap();
        assert_eq!(manifest, "2026-10-31/10-s-0.ndjson.zst\treplaced\n");
        pass(&args).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join(MANIFEST)).unwrap(),
            manifest,
            "done once"
        );
        assert!(!day.join("10-s-0.ndjson.zst.tmp").exists());
    }

    #[test]
    fn leaves_truncated_and_unsettled_files_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let cut = tmp.path().join("cut.ndjson.zst");
        zst(&cut, &body(), 1);
        let bytes = fs::read(&cut).unwrap();
        fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let fresh = tmp.path().join("fresh.ndjson.zst");
        zst(&fresh, &body(), 1);
        let fresh_bytes = fs::read(&fresh).unwrap();

        assert!(matches!(recompress(&cut, 19).unwrap(), Done::Truncated));
        assert_eq!(fs::read(&cut).unwrap(), &bytes[..bytes.len() / 2]);

        let args = Args {
            dir: tmp.path().to_path_buf(),
            watch: false,
            level: 19,
            settle_secs: 3600,
            every_secs: 0,
        };
        pass(&args).unwrap();
        assert_eq!(fs::read(&fresh).unwrap(), fresh_bytes, "not settled yet");
        assert!(!tmp.path().join(MANIFEST).exists());
    }
}
