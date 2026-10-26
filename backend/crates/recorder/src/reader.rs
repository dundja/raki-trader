//! Reader for raw files, tolerant of files that were never finished.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::record::Record;

/// Result of decoding one raw file.
pub struct ReadOutcome {
    pub records: Vec<Record>,
    /// The zstd frame ended early (process killed, disk full). Every line that
    /// was decoded before the cut is still returned.
    pub truncated: bool,
    /// Complete lines that were not valid records.
    pub bad_lines: usize,
}

/// Decodes `path`. An unexpected end of the zstd stream is not an error: it
/// sets `truncated` and keeps the lines decoded so far. A final line without
/// a trailing `\n` is dropped, as it may be cut short.
pub fn read_file(path: &Path) -> io::Result<ReadOutcome> {
    let mut decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                truncated = true;
                break;
            }
            Err(e) => return Err(e),
        }
    }

    let mut records = Vec::new();
    let mut bad_lines = 0;
    // Everything up to the last newline is whole lines; the rest is a partial line.
    if let Some(end) = data.iter().rposition(|&b| b == b'\n') {
        for line in data[..end].split(|&b| b == b'\n') {
            match serde_json::from_slice::<Record>(line) {
                Ok(rec) => records.push(rec),
                Err(_) => bad_lines += 1,
            }
        }
    }
    Ok(ReadOutcome {
        records,
        truncated,
        bad_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bad_lines_and_drops_unterminated_tail() {
        let good = r#"{"t":1,"s":"binance","c":0,"k":"text","d":"x"}"#;
        let text = format!("{good}\nnot json\n{good}\n{}", &good[..20]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.ndjson.zst");
        std::fs::write(&path, zstd::encode_all(text.as_bytes(), 3).unwrap()).unwrap();

        let out = read_file(&path).unwrap();
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.bad_lines, 1);
        assert!(!out.truncated);
    }
}
