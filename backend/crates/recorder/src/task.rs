//! Channel front-end and background task of the recorder.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::record::Record;
use crate::writer::RawWriter;

const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Counters the health snapshot reads.
#[derive(Default)]
pub struct RecorderStats {
    pub records: AtomicU64,
    /// Uncompressed line bytes written.
    pub bytes: AtomicU64,
    /// Records that could not be queued (channel full or closed).
    pub dropped: AtomicU64,
    /// Wall-clock epoch ms of the last successful flush.
    pub last_flush_ms: AtomicI64,
    pub current_file: Mutex<Option<String>>,
}

/// Cheap, cloneable, non-blocking handle feeds use to record frames.
#[derive(Clone)]
pub struct RecordTx {
    tx: mpsc::Sender<Record>,
    stats: Arc<RecorderStats>,
}

impl RecordTx {
    /// Queues `rec` without ever blocking. If the channel is full or the
    /// recorder is gone, the record is lost and `stats.dropped` is incremented.
    pub fn send(&self, rec: Record) {
        match self.tx.try_send(rec) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Closed(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn stats(&self) -> Arc<RecorderStats> {
        self.stats.clone()
    }
}

/// A bounded record channel. [`spawn_recorder`] consumes the receiver; tests of
/// feeds hold it to observe what was recorded.
pub fn record_channel(capacity: usize) -> (RecordTx, mpsc::Receiver<Record>) {
    let (tx, rx) = mpsc::channel(capacity);
    let stats = Arc::new(RecorderStats::default());
    (RecordTx { tx, stats }, rx)
}

/// Starts the recorder task. It writes records as they arrive, flushes every
/// second, and finishes the file when every [`RecordTx`] has been dropped. On
/// an I/O error it stops (later sends count as dropped) and returns the error.
pub fn spawn_recorder(
    dir: PathBuf,
    session: Uuid,
    capacity: usize,
) -> (RecordTx, tokio::task::JoinHandle<io::Result<()>>) {
    let (tx, rx) = record_channel(capacity);
    let stats = tx.stats();
    let handle = tokio::spawn(run(rx, RawWriter::new(dir, session), stats));
    (tx, handle)
}

async fn run(
    mut rx: mpsc::Receiver<Record>,
    mut writer: RawWriter,
    stats: Arc<RecorderStats>,
) -> io::Result<()> {
    let mut tick = tokio::time::interval(FLUSH_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut counted_bytes = 0;
    let result = loop {
        tokio::select! {
            rec = rx.recv() => match rec {
                Some(rec) => {
                    if let Err(e) = writer.write(&rec) {
                        break Err(e);
                    }
                    stats.records.fetch_add(1, Ordering::Relaxed);
                    let total = writer.bytes_written();
                    stats.bytes.fetch_add(total - counted_bytes, Ordering::Relaxed);
                    counted_bytes = total;
                    publish_file(&stats, &writer);
                }
                None => break Ok(()),
            },
            _ = tick.tick() => {
                if let Err(e) = writer.flush() {
                    break Err(e);
                }
                stats.last_flush_ms.store(raki_core::now_ms(), Ordering::Relaxed);
            }
        }
    };
    // Finish even after a write error: it salvages whatever is decodable.
    let finished = writer.finish();
    if result.is_ok() && finished.is_ok() {
        stats
            .last_flush_ms
            .store(raki_core::now_ms(), Ordering::Relaxed);
    }
    if let Some(e) = result.as_ref().err().or(finished.as_ref().err()) {
        tracing::error!(error = %e, "raw recorder stopped on I/O error");
    }
    result.and(finished)
}

fn publish_file(stats: &RecorderStats, writer: &RawWriter) {
    let file = writer.current_file().map(|p| p.display().to_string());
    *stats.current_file.lock().expect("stats mutex poisoned") = file;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use raki_core::Source;

    use super::*;
    use crate::reader::read_file;
    use crate::record::RecordKind;

    fn rec(i: i64) -> Record {
        Record {
            t: 1_793_012_399_000_000_000 + i,
            s: Source::Binance,
            c: 0,
            k: RecordKind::Text,
            d: format!("frame {i}"),
        }
    }

    #[tokio::test]
    async fn record_tx_counts_drops_when_full() {
        // The receiver is held but never polled, so the channel stays full.
        let (tx, _rx) = record_channel(2);
        for i in 0..5 {
            tx.send(rec(i));
        }
        assert_eq!(tx.stats().dropped.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn record_tx_counts_drops_when_closed() {
        let (tx, rx) = record_channel(2);
        drop(rx);
        tx.send(rec(0));
        assert_eq!(tx.stats().dropped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn spawned_recorder_flushes_and_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, handle) = spawn_recorder(dir.path().to_path_buf(), Uuid::now_v7(), 64);
        let stats = tx.stats();
        let recs: Vec<Record> = (0..10).map(rec).collect();
        for r in &recs {
            tx.send(r.clone());
        }
        drop(tx);
        handle.await.unwrap().unwrap();

        let file = stats.current_file.lock().unwrap().clone().unwrap();
        let out = read_file(std::path::Path::new(&file)).unwrap();
        assert_eq!(out.records, recs);
        assert!(!out.truncated);
        assert_eq!(stats.records.load(Ordering::Relaxed), 10);
        assert!(stats.bytes.load(Ordering::Relaxed) > 0);
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 0);
        assert!(stats.last_flush_ms.load(Ordering::Relaxed) > 0);
    }

    #[tokio::test(start_paused = true)]
    async fn spawned_recorder_flushes_every_second() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, handle) = spawn_recorder(dir.path().to_path_buf(), Uuid::now_v7(), 64);
        let stats = tx.stats();
        tx.send(rec(0));
        // Let the task receive the record and let the 1 s flush tick fire.
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        let file = stats.current_file.lock().unwrap().clone().unwrap();
        let out = read_file(std::path::Path::new(&file)).unwrap();
        assert!(out.truncated, "file is still open");
        assert_eq!(out.records, vec![rec(0)]);
        drop(tx);
        handle.await.unwrap().unwrap();
    }
}
