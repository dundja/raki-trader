//! The store writer: the one task that writes the engine loop's results to Postgres.
//!
//! The engine loop never awaits a database call: it hands every write to this task through a
//! bounded channel and moves on, so a slow or unreachable Postgres can no longer stall ticks,
//! drop feed events or freeze the live snapshot.
//!
//! A write that fails is kept and retried, never dropped on the first error:
//! - window records are kept per slug, newest wins (a record is the full state, and the store
//!   merges it without regressing anything), so a retry always writes the latest state;
//! - series rows, gaps, sleeps and control audits are kept in order, up to a bound;
//! - shadow rows (predictions, v0 evaluations and fills) likewise; their inserts are idempotent;
//! - after a failure the writer waits (1 s doubling to 10 s) and retries everything pending.
//!
//! [`WriterStats`] reports pending writes and the last error, which the health snapshot turns into
//! the `store_ok` safety check.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raki_core::{
    PredictionRow, SeriesRow, ShadowEvalRow, ShadowFillRow, Source, StoreStatus, TradingMode,
    WindowRecord, now_ms,
};
use raki_store::Store;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use uuid::Uuid;

/// One store call may take at most this long before it counts as failed.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(10);
/// Series rows kept while the store is failing (about an hour at 3 rows per second); the oldest
/// go first.
const MAX_PENDING_SERIES: usize = 12_000;
/// Gaps, sleeps and audits kept while the store is failing.
const MAX_PENDING_EVENTS: usize = 5_000;
/// Shadow rows kept while the store is failing (about an hour: ~40 predictions and ~16
/// evaluations per 5-minute window).
const MAX_PENDING_SHADOW: usize = 5_000;
/// The channel from the engine loop.
pub const WRITER_CAPACITY: usize = 50_000;

/// A control action for the audit log.
#[derive(Clone, Debug, PartialEq)]
pub struct Audit {
    pub at_ms: i64,
    pub action: &'static str,
    pub ok: bool,
    pub from: TradingMode,
    pub to: TradingMode,
    pub reason: Option<String>,
    pub error: Option<String>,
}

/// What the engine loop asks the writer to store.
#[derive(Debug)]
pub enum Write {
    Window(WindowRecord),
    Series(Vec<SeriesRow>),
    Gap {
        source: Source,
        at_ms: i64,
        reason: String,
    },
    /// `done` fires once the audit row is stored.
    Audit {
        audit: Audit,
        done: Option<oneshot::Sender<()>>,
    },
    Sleep {
        wake_ms: i64,
        slept_ms: i64,
    },
    Heartbeat {
        at_ms: i64,
    },
    Shadow(ShadowWrite),
}

/// A shadow row (docs/specs/2026-10-31-shadow-predictions.md).
#[derive(Clone, Debug, PartialEq)]
pub enum ShadowWrite {
    Prediction(PredictionRow),
    Eval(ShadowEvalRow),
    Fill(ShadowFillRow),
}

/// The store calls the writer makes. [`Store`] is the real one; tests use a fake that fails.
pub trait Sink: Send + Sync + 'static {
    fn upsert_window(&self, w: &WindowRecord) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn insert_series(&self, rows: &[SeriesRow]) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn insert_gap(
        &self,
        session: Uuid,
        source: Source,
        at_ms: i64,
        reason: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn insert_audit(&self, a: &Audit) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn record_sleep(
        &self,
        session: Uuid,
        wake_ms: i64,
        slept_ms: i64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn heartbeat(
        &self,
        session: Uuid,
        at_ms: i64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn insert_shadow(&self, w: &ShadowWrite) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl Sink for Store {
    async fn upsert_window(&self, w: &WindowRecord) -> anyhow::Result<()> {
        Store::upsert_window(self, w).await
    }

    async fn insert_series(&self, rows: &[SeriesRow]) -> anyhow::Result<()> {
        Store::insert_series(self, rows).await
    }

    async fn insert_gap(
        &self,
        session: Uuid,
        source: Source,
        at_ms: i64,
        reason: &str,
    ) -> anyhow::Result<()> {
        Store::insert_gap(self, session, source, at_ms, reason).await
    }

    async fn insert_audit(&self, a: &Audit) -> anyhow::Result<()> {
        Store::insert_audit(
            self,
            a.at_ms,
            a.action,
            a.ok,
            a.from,
            a.to,
            a.reason.as_deref(),
            a.error.as_deref(),
        )
        .await
    }

    async fn record_sleep(&self, session: Uuid, wake_ms: i64, slept_ms: i64) -> anyhow::Result<()> {
        Store::record_sleep(self, session, wake_ms, slept_ms).await
    }

    async fn heartbeat(&self, session: Uuid, at_ms: i64) -> anyhow::Result<()> {
        Store::heartbeat(self, session, at_ms).await
    }

    async fn insert_shadow(&self, w: &ShadowWrite) -> anyhow::Result<()> {
        match w {
            ShadowWrite::Prediction(p) => self.insert_prediction(p).await,
            ShadowWrite::Eval(e) => self.insert_shadow_eval(e).await,
            ShadowWrite::Fill(f) => self.insert_shadow_fill(f).await,
        }
    }
}

/// Shared with the health snapshot.
#[derive(Debug, Default)]
pub struct WriterStats {
    failing: AtomicBool,
    pending: AtomicU64,
    dropped: AtomicU64,
    last_ok_ms: AtomicI64,
    last_error: Mutex<Option<String>>,
}

impl WriterStats {
    pub fn status(&self) -> StoreStatus {
        let last_ok = self.last_ok_ms.load(Ordering::Relaxed);
        StoreStatus {
            ok: !self.failing.load(Ordering::Relaxed),
            pending_writes: self.pending.load(Ordering::Relaxed),
            dropped_writes: self.dropped.load(Ordering::Relaxed),
            last_ok_ms: (last_ok > 0).then_some(last_ok),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    /// A write that never reached the writer (its channel was full) or was evicted from a full
    /// retry buffer.
    pub fn count_dropped(&self, n: u64) {
        self.dropped.fetch_add(n, Ordering::Relaxed);
    }
}

/// The engine loop's handle. Sending never blocks.
#[derive(Clone)]
pub struct WriterTx {
    tx: mpsc::Sender<Write>,
    stats: Arc<WriterStats>,
}

impl WriterTx {
    pub fn send(&self, w: Write) {
        if let Err(e) = self.tx.try_send(w) {
            self.stats.count_dropped(1);
            tracing::error!(error = %e, "store writer channel full; write dropped");
        }
    }

    pub fn stats(&self) -> Arc<WriterStats> {
        self.stats.clone()
    }
}

/// Spawns the writer for `session`. It runs until every [`WriterTx`] is dropped, then makes a
/// last attempt to store everything pending and returns.
pub fn spawn_writer<S: Sink>(sink: S, session: Uuid) -> (WriterTx, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(WRITER_CAPACITY);
    let stats = Arc::new(WriterStats::default());
    let writer = Writer {
        sink,
        session,
        stats: stats.clone(),
        windows: BTreeMap::new(),
        series: VecDeque::new(),
        events: VecDeque::new(),
        shadow: VecDeque::new(),
        heartbeat: None,
        retry_at: None,
        retry_delay: RETRY_MIN,
    };
    let handle = tokio::spawn(writer.run(rx));
    (WriterTx { tx, stats }, handle)
}

/// Writes kept in order: gaps, sleeps and audits.
#[derive(Debug)]
enum Event {
    Gap {
        source: Source,
        at_ms: i64,
        reason: String,
    },
    Audit {
        audit: Audit,
        done: Option<oneshot::Sender<()>>,
    },
    Sleep {
        wake_ms: i64,
        slept_ms: i64,
    },
}

struct Writer<S> {
    sink: S,
    session: Uuid,
    stats: Arc<WriterStats>,
    /// Pending window upserts, newest record per slug.
    windows: BTreeMap<String, WindowRecord>,
    series: VecDeque<SeriesRow>,
    events: VecDeque<Event>,
    shadow: VecDeque<ShadowWrite>,
    heartbeat: Option<i64>,
    /// Set after a failure: nothing is attempted before it.
    retry_at: Option<Instant>,
    retry_delay: Duration,
}

impl<S: Sink> Writer<S> {
    async fn run(mut self, mut rx: mpsc::Receiver<Write>) {
        loop {
            let retry_at = self.retry_at;
            let wait = async move {
                match retry_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                biased;
                msg = rx.recv() => match msg {
                    Some(w) => self.absorb(w),
                    None => break,
                },
                () = wait => self.retry_at = None,
            }
            while let Ok(w) = rx.try_recv() {
                self.absorb(w);
            }
            // A steady stream of writes must not starve a retry that is due.
            if self.retry_at.is_some_and(|t| Instant::now() >= t) {
                self.retry_at = None;
            }
            if self.retry_at.is_none() {
                self.flush().await;
            }
        }
        // Shutting down: one last attempt, whatever the retry schedule says.
        self.flush().await;
        let left = self.pending();
        if left > 0 {
            tracing::error!(
                pending = left,
                "store writer stopped with writes not stored"
            );
        }
    }

    fn absorb(&mut self, w: Write) {
        match w {
            Write::Window(rec) => {
                self.windows.insert(rec.slug.clone(), rec);
            }
            Write::Series(rows) => {
                self.series.extend(rows);
                let over = self.series.len().saturating_sub(MAX_PENDING_SERIES);
                if over > 0 {
                    self.series.drain(..over);
                    self.stats.count_dropped(over as u64);
                    tracing::error!(
                        rows = over,
                        "store failing too long: oldest series rows dropped"
                    );
                }
            }
            Write::Gap {
                source,
                at_ms,
                reason,
            } => self.push_event(Event::Gap {
                source,
                at_ms,
                reason,
            }),
            Write::Audit { audit, done } => self.push_event(Event::Audit { audit, done }),
            Write::Sleep { wake_ms, slept_ms } => {
                self.push_event(Event::Sleep { wake_ms, slept_ms })
            }
            Write::Heartbeat { at_ms } => self.heartbeat = Some(at_ms),
            Write::Shadow(w) => {
                if self.shadow.len() >= MAX_PENDING_SHADOW {
                    self.shadow.pop_front();
                    self.stats.count_dropped(1);
                    tracing::error!("store failing too long: oldest shadow row dropped");
                }
                self.shadow.push_back(w);
            }
        }
        self.publish_pending();
    }

    fn push_event(&mut self, e: Event) {
        if self.events.len() >= MAX_PENDING_EVENTS {
            self.events.pop_front();
            self.stats.count_dropped(1);
            tracing::error!("store failing too long: oldest gap/audit write dropped");
        }
        self.events.push_back(e);
    }

    fn pending(&self) -> u64 {
        (self.windows.len() + self.series.len() + self.events.len() + self.shadow.len()) as u64
            + u64::from(self.heartbeat.is_some())
    }

    fn publish_pending(&self) {
        self.stats.pending.store(self.pending(), Ordering::Relaxed);
    }

    /// Stores everything pending, stopping at the first failure (the rest waits for the retry).
    async fn flush(&mut self) {
        let result = self.try_flush().await;
        self.publish_pending();
        match result {
            Ok(()) => {
                if self.stats.failing.swap(false, Ordering::Relaxed) {
                    tracing::info!("store writes recovered");
                }
                self.stats.last_ok_ms.store(now_ms(), Ordering::Relaxed);
                self.retry_delay = RETRY_MIN;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if !self.stats.failing.swap(true, Ordering::Relaxed) {
                    tracing::error!(error = %msg, pending = self.pending(), "store write failed; retrying");
                }
                *self
                    .stats
                    .last_error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(msg);
                self.retry_at = Some(Instant::now() + self.retry_delay);
                self.retry_delay = (self.retry_delay * 2).min(RETRY_MAX);
            }
        }
    }

    async fn try_flush(&mut self) -> anyhow::Result<()> {
        while let Some(e) = self.events.front() {
            match e {
                Event::Gap {
                    source,
                    at_ms,
                    reason,
                } => {
                    call(self.sink.insert_gap(self.session, *source, *at_ms, reason)).await?;
                }
                Event::Audit { audit, .. } => call(self.sink.insert_audit(audit)).await?,
                Event::Sleep { wake_ms, slept_ms } => {
                    call(self.sink.record_sleep(self.session, *wake_ms, *slept_ms)).await?;
                }
            }
            if let Some(Event::Audit {
                done: Some(done), ..
            }) = self.events.pop_front()
            {
                let _ = done.send(());
            }
        }
        while let Some((slug, rec)) = self.windows.pop_first() {
            if let Err(e) = call(self.sink.upsert_window(&rec)).await {
                self.windows.insert(slug, rec);
                return Err(e);
            }
        }
        if !self.series.is_empty() {
            let rows: Vec<SeriesRow> = self.series.iter().cloned().collect();
            call(self.sink.insert_series(&rows)).await?;
            self.series.clear();
        }
        // Windows first: a fill or evaluation is read back joined with its window.
        while let Some(w) = self.shadow.front() {
            call(self.sink.insert_shadow(w)).await?;
            self.shadow.pop_front();
        }
        if let Some(at_ms) = self.heartbeat {
            call(self.sink.heartbeat(self.session, at_ms)).await?;
            self.heartbeat = None;
        }
        Ok(())
    }
}

async fn call(f: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    match tokio::time::timeout(CALL_TIMEOUT, f).await {
        Ok(r) => r,
        Err(_) => anyhow::bail!("store call timed out after {CALL_TIMEOUT:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use raki_core::{Asset, WindowStatus};

    use super::*;

    /// Fails every call while `down` is set; records what it stored.
    #[derive(Clone, Default)]
    struct Fake {
        down: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        windows: Arc<Mutex<Vec<WindowRecord>>>,
        series: Arc<Mutex<Vec<SeriesRow>>>,
        gaps: Arc<Mutex<Vec<String>>>,
        audits: Arc<Mutex<Vec<&'static str>>>,
        sleeps: Arc<Mutex<Vec<(i64, i64)>>>,
        heartbeats: Arc<Mutex<Vec<i64>>>,
        shadow: Arc<Mutex<Vec<ShadowWrite>>>,
    }

    impl Fake {
        fn check(&self) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                anyhow::bail!("connection refused");
            }
            Ok(())
        }
    }

    impl Sink for Fake {
        async fn upsert_window(&self, w: &WindowRecord) -> anyhow::Result<()> {
            self.check()?;
            self.windows.lock().unwrap().push(w.clone());
            Ok(())
        }
        async fn insert_series(&self, rows: &[SeriesRow]) -> anyhow::Result<()> {
            self.check()?;
            self.series.lock().unwrap().extend_from_slice(rows);
            Ok(())
        }
        async fn insert_gap(&self, _: Uuid, _: Source, _: i64, reason: &str) -> anyhow::Result<()> {
            self.check()?;
            self.gaps.lock().unwrap().push(reason.to_owned());
            Ok(())
        }
        async fn insert_audit(&self, a: &Audit) -> anyhow::Result<()> {
            self.check()?;
            self.audits.lock().unwrap().push(a.action);
            Ok(())
        }
        async fn record_sleep(&self, _: Uuid, wake_ms: i64, slept_ms: i64) -> anyhow::Result<()> {
            self.check()?;
            self.sleeps.lock().unwrap().push((wake_ms, slept_ms));
            Ok(())
        }
        async fn heartbeat(&self, _: Uuid, at_ms: i64) -> anyhow::Result<()> {
            self.check()?;
            self.heartbeats.lock().unwrap().push(at_ms);
            Ok(())
        }
        async fn insert_shadow(&self, w: &ShadowWrite) -> anyhow::Result<()> {
            self.check()?;
            self.shadow.lock().unwrap().push(w.clone());
            Ok(())
        }
    }

    fn rec(slug: &str, status: WindowStatus) -> WindowRecord {
        WindowRecord {
            slug: slug.into(),
            asset: Asset::Btc,
            start_ts: 0,
            end_ts: 300,
            event_id: None,
            condition_id: None,
            up_token: None,
            down_token: None,
            price_to_beat_ours: None,
            final_twap_ours: None,
            final_twap_source: None,
            outcome_ours: None,
            price_to_beat_gamma: None,
            final_price_gamma: None,
            twap_match: None,
            winner: None,
            resolved_at: None,
            resolution_seen_ms: None,
            twap_source: None,
            coverage_secs: 0,
            status,
            session_id: Uuid::nil().to_string(),
            updated_ms: 0,
        }
    }

    fn series(ts: i64) -> SeriesRow {
        SeriesRow {
            slug: "s".into(),
            ts,
            twap: None,
            chainlink: None,
            binance_mid_usd: None,
            coinbase_mid: None,
            up_bid: None,
            up_ask: None,
            down_bid: None,
            down_ask: None,
        }
    }

    fn prediction(slug: &str) -> ShadowWrite {
        ShadowWrite::Prediction(PredictionRow {
            slug: slug.into(),
            asset: Asset::Btc,
            checkpoint_s: 30,
            ts_ms: 1,
            p_up: 0.9,
            mu: 1.0,
            sd: 0.1,
            sigma: 0.01,
            ptb: 0.9,
            spot: 1.0,
            twap: None,
            binance_mid_usd: None,
            coinbase_mid: None,
            up_bid: None,
            up_ask: None,
            down_bid: None,
            down_ask: None,
            model_id: "m".into(),
            session_id: Uuid::nil().to_string(),
        })
    }

    async fn eventually(mut f: impl FnMut() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not met");
    }

    #[tokio::test(start_paused = true)]
    async fn failed_writes_are_kept_and_retried_until_stored() {
        let fake = Fake::default();
        fake.down.store(true, Ordering::SeqCst);
        let (tx, handle) = spawn_writer(fake.clone(), Uuid::nil());
        let stats = tx.stats();

        // The final transition of a window is written while Postgres is down.
        tx.send(Write::Window(rec("w", WindowStatus::Resolved)));
        tx.send(Write::Window(rec("w", WindowStatus::Complete)));
        tx.send(Write::Series(vec![series(1), series(2)]));
        tx.send(Write::Gap {
            source: Source::Binance,
            at_ms: 5,
            reason: "reconnect".into(),
        });
        tx.send(Write::Sleep {
            wake_ms: 9,
            slept_ms: 4,
        });
        tx.send(Write::Heartbeat { at_ms: 10 });
        tx.send(Write::Shadow(prediction("a")));
        tx.send(Write::Shadow(prediction("b")));
        eventually(|| !stats.status().ok).await;
        let st = stats.status();
        assert_eq!(st.pending_writes, 8, "{st:?}");
        assert!(
            st.last_error
                .as_deref()
                .is_some_and(|e| e.contains("refused"))
        );

        // Postgres comes back: everything lands on a retry, the window as its newest record.
        fake.down.store(false, Ordering::SeqCst);
        eventually(|| stats.status().ok && stats.status().pending_writes == 0).await;
        let windows = fake.windows.lock().unwrap().clone();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].status, WindowStatus::Complete);
        assert_eq!(fake.series.lock().unwrap().len(), 2);
        assert_eq!(*fake.gaps.lock().unwrap(), ["reconnect"]);
        assert_eq!(*fake.sleeps.lock().unwrap(), [(9, 4)]);
        assert_eq!(*fake.heartbeats.lock().unwrap(), [10]);
        assert_eq!(
            *fake.shadow.lock().unwrap(),
            [prediction("a"), prediction("b")],
            "shadow rows in order"
        );
        assert!(stats.status().last_ok_ms.is_some());

        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn retries_back_off_and_the_last_flush_happens_on_close() {
        let fake = Fake::default();
        fake.down.store(true, Ordering::SeqCst);
        let (tx, handle) = spawn_writer(fake.clone(), Uuid::nil());
        tx.send(Write::Window(rec("w", WindowStatus::Ended)));
        let stats = tx.stats();
        eventually(|| !stats.status().ok).await;
        // 1 s, 2 s, 4 s, then 8 s between attempts: 4 attempts in the first 7 s.
        let before = fake.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(7_100)).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst) - before, 3);

        // Closing the channel makes one last attempt at once.
        fake.down.store(false, Ordering::SeqCst);
        drop(tx);
        handle.await.unwrap();
        assert_eq!(fake.windows.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn audit_ack_fires_once_stored() {
        let fake = Fake::default();
        let (tx, handle) = spawn_writer(fake.clone(), Uuid::nil());
        let (done, stored) = oneshot::channel();
        tx.send(Write::Audit {
            audit: Audit {
                at_ms: 1,
                action: "pause",
                ok: true,
                from: TradingMode::Running,
                to: TradingMode::Paused,
                reason: None,
                error: None,
            },
            done: Some(done),
        });
        tokio::time::timeout(Duration::from_secs(2), stored)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*fake.audits.lock().unwrap(), ["pause"]);
        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn series_buffer_is_bounded() {
        let fake = Fake::default();
        fake.down.store(true, Ordering::SeqCst);
        let (tx, handle) = spawn_writer(fake.clone(), Uuid::nil());
        let stats = tx.stats();
        let rows: Vec<SeriesRow> = (0..MAX_PENDING_SERIES as i64 + 10).map(series).collect();
        tx.send(Write::Series(rows));
        eventually(|| stats.status().dropped_writes == 10).await;
        fake.down.store(false, Ordering::SeqCst);
        drop(tx);
        handle.await.unwrap();
        let stored = fake.series.lock().unwrap();
        assert_eq!(stored.len(), MAX_PENDING_SERIES);
        assert_eq!(stored[0].ts, 10, "the oldest rows went first");
    }
}
