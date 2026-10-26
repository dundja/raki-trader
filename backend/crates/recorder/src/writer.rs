//! Hourly-rotated zstd NDJSON writer.
//!
//! Each file is `<dir>/raw/YYYY-MM-DD/HH-<session8>-<n>.ndjson.zst`. A file is
//! only ever created with `create_new`, so an existing file is never appended
//! to: after a crash the zstd frame of the old file is unfinished and cannot be
//! continued, so the next writer takes the next free `n`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Timelike, Utc};
use uuid::Uuid;
use zstd::stream::write::Encoder;

use crate::record::Record;

const ZSTD_LEVEL: i32 = 3;
const NANOS_PER_HOUR: i64 = 3_600 * 1_000_000_000;

struct Open {
    encoder: Encoder<'static, File>,
    path: PathBuf,
    /// Hours since the epoch (UTC) that this file covers.
    hour: i64,
}

/// Writes [`Record`]s as one JSON line each into hourly zstd files.
pub struct RawWriter {
    dir: PathBuf,
    session8: String,
    open: Option<Open>,
    bytes_written: u64,
}

impl RawWriter {
    /// Files are opened lazily, on the first [`RawWriter::write`].
    pub fn new(dir: PathBuf, session: Uuid) -> RawWriter {
        let session8 = session.simple().to_string()[..8].to_owned();
        RawWriter {
            dir,
            session8,
            open: None,
            bytes_written: 0,
        }
    }

    /// Appends one JSON line. Rotates to a new file when the UTC hour of
    /// `rec.t` differs from the hour of the current file.
    pub fn write(&mut self, rec: &Record) -> io::Result<()> {
        let hour = rec.t.div_euclid(NANOS_PER_HOUR);
        if self.open.as_ref().is_some_and(|o| o.hour != hour) {
            self.finish()?;
        }
        if self.open.is_none() {
            self.open = Some(self.open_file(rec.t, hour)?);
        }
        let mut line = serde_json::to_vec(rec).map_err(io::Error::other)?;
        line.push(b'\n');
        let open = self.open.as_mut().expect("opened above");
        open.encoder.write_all(&line)?;
        self.bytes_written += line.len() as u64;
        Ok(())
    }

    /// Makes everything written so far decodable (zstd flush, then file flush).
    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(open) = self.open.as_mut() {
            open.encoder.flush()?;
            open.encoder.get_mut().flush()?;
        }
        Ok(())
    }

    /// Ends the zstd frame, syncs and closes the current file, if any.
    pub fn finish(&mut self) -> io::Result<()> {
        if let Some(open) = self.open.take() {
            let file = open.encoder.finish()?;
            file.sync_all()?;
        }
        Ok(())
    }

    /// The file being written, if one is open.
    pub fn current_file(&self) -> Option<PathBuf> {
        self.open.as_ref().map(|o| o.path.clone())
    }

    /// Total uncompressed line bytes written so far (all files).
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    fn open_file(&self, t_ns: i64, hour: i64) -> io::Result<Open> {
        let at = DateTime::<Utc>::from_timestamp(
            t_ns.div_euclid(1_000_000_000),
            t_ns.rem_euclid(1_000_000_000) as u32,
        )
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "record time out of range"))?;
        let day_dir = self.dir.join("raw").join(at.format("%Y-%m-%d").to_string());
        fs::create_dir_all(&day_dir)?;
        let (file, path) = create_new_numbered(&day_dir, at.hour(), &self.session8)?;
        let mut encoder = Encoder::new(file, ZSTD_LEVEL)?;
        encoder.include_checksum(true)?;
        Ok(Open {
            encoder,
            path,
            hour,
        })
    }
}

/// Creates `HH-<session8>-<n>.ndjson.zst` with the lowest `n` that is free.
fn create_new_numbered(day_dir: &Path, hour: u32, session8: &str) -> io::Result<(File, PathBuf)> {
    for n in 0u32.. {
        let path = day_dir.join(format!("{hour:02}-{session8}-{n}.ndjson.zst"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("u32 range exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::read_file;
    use crate::record::RecordKind;
    use raki_core::Source;

    const SESSION: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

    /// 2026-10-26T10:59:59Z in epoch ns.
    const T_10_59_59: i64 = 1_793_012_399_000_000_000;
    /// 2026-10-26T11:00:00Z in epoch ns.
    const T_11_00_00: i64 = T_10_59_59 + 1_000_000_000;

    fn rec(t: i64, d: impl Into<String>) -> Record {
        Record {
            t,
            s: Source::ClobMarket,
            c: 1,
            k: RecordKind::Text,
            d: d.into(),
        }
    }

    #[test]
    fn roundtrip_preserves_exact_text() {
        let dir = tempfile::tempdir().unwrap();
        let big = "x".repeat(100 * 1024);
        let recs = vec![
            rec(
                T_10_59_59,
                "{\"a\":\"quo\\\"te\",\n\"b\":\"line\\nbreak\"}\r\n",
            ),
            rec(
                T_10_59_59 + 1,
                "unicode: \u{1F600} \u{00e9} \u{4e2d}\u{6587} \u{2028}",
            ),
            rec(T_10_59_59 + 2, big),
        ];
        let mut w = RawWriter::new(dir.path().to_path_buf(), SESSION);
        for r in &recs {
            w.write(r).unwrap();
        }
        let path = w.current_file().unwrap();
        w.finish().unwrap();

        let out = read_file(&path).unwrap();
        assert_eq!(out.records, recs);
        assert!(!out.truncated);
        assert_eq!(out.bad_lines, 0);
    }

    #[test]
    fn rotates_on_utc_hour_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = RawWriter::new(dir.path().to_path_buf(), SESSION);
        w.write(&rec(T_10_59_59, "a")).unwrap();
        let first = w.current_file().unwrap();
        w.write(&rec(T_11_00_00, "b")).unwrap();
        let second = w.current_file().unwrap();
        w.finish().unwrap();

        let day = dir.path().join("raw").join("2026-10-26");
        assert_eq!(first, day.join("10-01234567-0.ndjson.zst"));
        assert_eq!(second, day.join("11-01234567-0.ndjson.zst"));
        // The first file was finished on rotation.
        let out = read_file(&first).unwrap();
        assert_eq!(out.records.len(), 1);
        assert!(!out.truncated);
        assert_eq!(read_file(&second).unwrap().records[0].d, "b");
    }

    #[test]
    fn reader_recovers_flushed_lines_from_truncated_file() {
        let dir = tempfile::tempdir().unwrap();
        let recs: Vec<Record> = (0..100)
            .map(|i| rec(T_10_59_59 + i, format!("frame-{i}-{}", "p".repeat(200))))
            .collect();
        let mut w = RawWriter::new(dir.path().to_path_buf(), SESSION);
        for (i, r) in recs.iter().enumerate() {
            w.write(r).unwrap();
            if i == 59 {
                w.flush().unwrap();
            }
        }
        w.flush().unwrap();
        let path = w.current_file().unwrap();
        // Simulate a kill: no finish, and the tail of the file is lost.
        let len = std::fs::metadata(&path).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 7)
            .unwrap();

        let out = read_file(&path).unwrap();
        assert!(out.truncated);
        assert!(out.records.len() >= 60, "got {}", out.records.len());
        assert_eq!(out.bad_lines, 0);
        assert_eq!(out.records[..], recs[..out.records.len()]);
    }

    #[test]
    fn reader_recovers_flushed_lines_when_never_finished() {
        let dir = tempfile::tempdir().unwrap();
        let recs: Vec<Record> = (0..10)
            .map(|i| rec(T_10_59_59 + i, format!("f{i}")))
            .collect();
        let mut w = RawWriter::new(dir.path().to_path_buf(), SESSION);
        for r in &recs {
            w.write(r).unwrap();
        }
        w.flush().unwrap();
        let out = read_file(&w.current_file().unwrap()).unwrap();
        assert!(out.truncated);
        assert_eq!(out.records, recs);
    }

    #[test]
    fn writer_never_appends_to_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("raw").join("2026-10-26");
        std::fs::create_dir_all(&day).unwrap();
        let existing = day.join("10-01234567-0.ndjson.zst");
        std::fs::write(&existing, b"precious").unwrap();

        let mut w = RawWriter::new(dir.path().to_path_buf(), SESSION);
        w.write(&rec(T_10_59_59, "a")).unwrap();
        let path = w.current_file().unwrap();
        w.finish().unwrap();

        assert_eq!(path, day.join("10-01234567-1.ndjson.zst"));
        assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
        assert_eq!(read_file(&path).unwrap().records.len(), 1);
    }
}
