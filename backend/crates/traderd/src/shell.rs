//! The engine loop: the single task that owns the [`Engine`] and carries out what it asks for.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use raki_api::{ControlCommand, ControlMsg};
use raki_core::{
    ControlResponse, FeedEvent, HealthSnapshot, LiveSnapshot, RecorderStatus, SessionInfo, Source,
    now_ms,
};
use raki_engine::engine::{Engine, EngineOutput, Request};
use raki_feeds::rest::PolyRest;
use raki_feeds::status::FeedStatusRegistry;
use raki_recorder::RecorderStats;
use raki_store::Store;
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::health::{self, HealthState};

const TICK: Duration = Duration::from_secs(1);
/// The recorder safety check looks at drops over this trailing window.
const DROP_WINDOW_MS: i64 = 60_000;

/// This app run.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: Uuid,
    pub started_ms: i64,
}

impl Session {
    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.to_string(),
            started_ms: self.started_ms,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// The channels the engine loop reads and writes.
pub struct LoopIo {
    /// REST results are fed back on the events channel, so the loop holds a sender too.
    pub events_tx: mpsc::Sender<FeedEvent>,
    pub events_rx: mpsc::Receiver<FeedEvent>,
    pub control_rx: mpsc::Receiver<ControlMsg>,
    pub desired_tx: watch::Sender<BTreeSet<String>>,
    pub live_tx: watch::Sender<LiveSnapshot>,
}

/// What the health snapshot and the safety checks are built from.
pub struct HealthInputs {
    pub feeds: Arc<FeedStatusRegistry>,
    pub state: Arc<HealthState>,
    pub recorder: Arc<RecorderStats>,
}

impl HealthInputs {
    fn recorder_status(&self) -> RecorderStatus {
        let r = &self.recorder;
        let last_flush = r.last_flush_ms.load(Ordering::Relaxed);
        RecorderStatus {
            file: r
                .current_file
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            records: r.records.load(Ordering::Relaxed),
            bytes: r.bytes.load(Ordering::Relaxed),
            dropped: r.dropped.load(Ordering::Relaxed),
            last_flush_ms: (last_flush > 0).then_some(last_flush),
        }
    }

    fn snapshot(&self, book_resyncs: u32, dropped_last_min: u64, now_ms: i64) -> HealthSnapshot {
        let feeds = self.feeds.snapshot(now_ms);
        let clock = self.state.clock();
        let geoblock = self.state.geo();
        let recorder = self.recorder_status();
        let safety = health::safety(&feeds, &clock, &geoblock, &recorder, dropped_last_min);
        HealthSnapshot {
            feeds,
            clock,
            geoblock,
            recorder,
            sleep_events: self.state.sleep_events(),
            book_resyncs,
            safety,
        }
    }
}

/// The live snapshot as of now, without touching the control state. `main` seeds the `watch`
/// channel with it; the loop then publishes a fresh one every tick.
pub fn live_snapshot(
    engine: &mut Engine,
    session: &Session,
    health: &HealthInputs,
    dropped_last_min: u64,
    now_ms: i64,
) -> LiveSnapshot {
    LiveSnapshot {
        generated_ms: now_ms,
        session: session.info(),
        health: health.snapshot(engine.book_resyncs(), dropped_last_min, now_ms),
        control: engine.control().state(),
        windows: engine.live_windows(now_ms),
    }
}

/// How much the recorder's drop counter grew over the trailing minute.
struct DropWindow {
    /// `(at_ms, dropped_total)`, oldest first. The front is the baseline.
    samples: VecDeque<(i64, u64)>,
}

impl DropWindow {
    fn new(now_ms: i64, total: u64) -> Self {
        DropWindow {
            samples: VecDeque::from([(now_ms, total)]),
        }
    }

    fn update(&mut self, now_ms: i64, total: u64) -> u64 {
        self.samples.push_back((now_ms, total));
        // Keep the newest sample that is at least a minute old as the baseline.
        while self
            .samples
            .get(1)
            .is_some_and(|&(t, _)| t <= now_ms - DROP_WINDOW_MS)
        {
            self.samples.pop_front();
        }
        let base = self.samples.front().map_or(total, |&(_, d)| d);
        total.saturating_sub(base)
    }
}

/// Runs until `shutdown`: one `select!` over feed events, control messages and a 1 s tick.
///
/// - Engine outputs are carried out here: series rows are batched and flushed each tick, windows
///   are upserted, REST requests run as spawned tasks whose results come back as events, the
///   desired CLOB tokens go to the CLOB runner, and a book resync is logged and stored as a gap.
/// - Every tick computes the safety checks, feeds them to the control state machine and publishes
///   a [`LiveSnapshot`].
/// - Control messages are applied, audited and answered.
/// - A `FeedReset` after a source's first one is a reconnect: it is stored as a feed gap.
///
/// Store errors are logged and never stop the loop.
pub async fn run_engine_loop(
    engine: Engine,
    store: Store,
    rest: PolyRest,
    session: Session,
    io: LoopIo,
    health: HealthInputs,
    shutdown: CancellationToken,
) {
    let LoopIo {
        events_tx,
        mut events_rx,
        mut control_rx,
        desired_tx,
        live_tx,
    } = io;
    let dropped = health.recorder.dropped.load(Ordering::Relaxed);
    let mut shell = Shell {
        engine,
        store,
        rest: Arc::new(rest),
        session,
        events_tx,
        desired_tx,
        live_tx,
        health,
        shutdown: shutdown.clone(),
        series: Vec::new(),
        resets_seen: HashSet::new(),
        drops: DropWindow::new(now_ms(), dropped),
    };
    let mut tick = tokio::time::interval(TICK);
    // After a sleep, one tick rather than a burst: a burst would inflate coverage counts.
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            Some(msg) = control_rx.recv() => shell.on_control(msg).await,
            _ = tick.tick() => shell.on_tick().await,
            Some(ev) = events_rx.recv() => shell.on_event(ev).await,
        }
    }
    shell.flush_series().await;
    tracing::info!("engine loop stopped");
}

struct Shell {
    engine: Engine,
    store: Store,
    rest: Arc<PolyRest>,
    session: Session,
    events_tx: mpsc::Sender<FeedEvent>,
    desired_tx: watch::Sender<BTreeSet<String>>,
    live_tx: watch::Sender<LiveSnapshot>,
    health: HealthInputs,
    shutdown: CancellationToken,
    /// Series rows waiting for the next flush.
    series: Vec<raki_core::SeriesRow>,
    /// Sources whose first `FeedReset` (the initial connect) has been seen.
    resets_seen: HashSet<Source>,
    drops: DropWindow,
}

impl Shell {
    async fn on_event(&mut self, ev: FeedEvent) {
        let now = now_ms();
        if let FeedEvent::FeedReset { source } = ev
            && !self.resets_seen.insert(source)
        {
            self.gap(source, now, "reconnect").await;
        }
        let outs = self.engine.handle(ev, now);
        self.execute(outs, now).await;
    }

    async fn on_tick(&mut self) {
        let now = now_ms();
        let outs = self.engine.tick(now);
        self.execute(outs, now).await;
        self.flush_series().await;
        let health = self.health_snapshot(now);
        self.engine.control().on_safety(&health.safety, now);
        self.publish(health, now);
    }

    fn health_snapshot(&mut self, now: i64) -> HealthSnapshot {
        let dropped = self.health.recorder.dropped.load(Ordering::Relaxed);
        let dropped_last_min = self.drops.update(now, dropped);
        self.health
            .snapshot(self.engine.book_resyncs(), dropped_last_min, now)
    }

    fn publish(&mut self, health: HealthSnapshot, now: i64) {
        let snapshot = LiveSnapshot {
            generated_ms: now,
            session: self.session.info(),
            health,
            control: self.engine.control().state(),
            windows: self.engine.live_windows(now),
        };
        // `send_replace` keeps the value even while no API client is subscribed.
        self.live_tx.send_replace(snapshot);
    }

    async fn on_control(&mut self, msg: ControlMsg) {
        let now = now_ms();
        let from = self.engine.control().state().mode;
        let (action, reason, result) = match msg.cmd {
            ControlCommand::Pause { reason } => {
                let state = self.engine.control().pause(reason.clone(), now);
                ("pause", reason, Ok(state))
            }
            ControlCommand::Kill { reason } => {
                let state = self.engine.control().kill(reason.clone(), now);
                ("kill", reason, Ok(state))
            }
            ControlCommand::Resume { acknowledge } => {
                let safety = self.health_snapshot(now).safety;
                let result = self.engine.control().resume(&safety, acknowledge, now);
                (
                    "resume",
                    acknowledge.then(|| "acknowledged".to_owned()),
                    result,
                )
            }
        };
        let resp = match result {
            Ok(state) => ControlResponse {
                ok: true,
                state,
                error: None,
            },
            Err(error) => ControlResponse {
                ok: false,
                state: self.engine.control().state(),
                error: Some(error),
            },
        };
        match &resp.error {
            None => tracing::info!(action, to = ?resp.state.mode, ?reason, "control"),
            Some(error) => tracing::warn!(action, %error, "control refused"),
        }
        // Audited before the answer: the client never sees an unaudited action.
        if let Err(e) = self
            .store
            .insert_audit(
                now,
                action,
                resp.ok,
                from,
                resp.state.mode,
                reason.as_deref(),
                resp.error.as_deref(),
            )
            .await
        {
            tracing::error!(error = ?e, "store: control audit");
        }
        // The API gave up waiting if the receiver is gone; the action still stands.
        let _ = msg.reply.send(resp);
        let health = self.health_snapshot(now);
        self.publish(health, now);
    }

    async fn execute(&mut self, outs: Vec<EngineOutput>, now: i64) {
        for out in outs {
            match out {
                EngineOutput::Series(rows) => self.series.extend(rows),
                EngineOutput::Window(rec) => {
                    if let Err(e) = self.store.upsert_window(&rec).await {
                        tracing::error!(error = ?e, slug = %rec.slug, "store: upsert window");
                    }
                }
                EngineOutput::Requests(reqs) => {
                    for req in reqs {
                        self.spawn_request(req);
                    }
                }
                EngineOutput::DesiredTokens(tokens) => {
                    tracing::info!(tokens = tokens.len(), "desired clob tokens changed");
                    // `send_replace` stores the set even when the CLOB runner is disabled.
                    self.desired_tx.send_replace(tokens);
                }
                EngineOutput::BookResync { asset_id } => {
                    tracing::warn!(%asset_id, "book snapshot disagreed with our book; resynced");
                    let reason = format!("book resync {asset_id}");
                    self.gap(Source::ClobMarket, now, &reason).await;
                }
            }
        }
    }

    async fn flush_series(&mut self) {
        if self.series.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.series);
        if let Err(e) = self.store.insert_series(&rows).await {
            tracing::error!(error = ?e, rows = rows.len(), "store: insert series");
        }
    }

    async fn gap(&self, source: Source, at_ms: i64, reason: &str) {
        if let Err(e) = self
            .store
            .insert_gap(self.session.id, source, at_ms, reason)
            .await
        {
            tracing::error!(error = ?e, source = source.as_str(), "store: insert gap");
        }
    }

    /// Runs the REST call off the loop; its result comes back as a [`FeedEvent`].
    fn spawn_request(&self, req: Request) {
        let rest = self.rest.clone();
        let events = self.events_tx.clone();
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            // Dropping out on shutdown also drops this task's recorder handle promptly.
            let ev = tokio::select! {
                () = shutdown.cancelled() => return,
                ev = fetch(&rest, req) => ev,
            };
            if let Some(ev) = ev {
                tokio::select! {
                    () = shutdown.cancelled() => {}
                    _ = events.send(ev) => {}
                }
            }
        });
    }
}

async fn fetch(rest: &PolyRest, req: Request) -> Option<FeedEvent> {
    match req {
        Request::FetchGamma { slug } => match rest.gamma_window(&slug).await {
            Ok(window) => Some(FeedEvent::GammaFetched {
                slug,
                window,
                recv_ms: now_ms(),
            }),
            Err(e) => {
                tracing::warn!(%slug, error = %e, "gamma fetch failed");
                Some(FeedEvent::GammaFailed { slug })
            }
        },
        Request::FetchResolution { slug, condition_id } => {
            match rest.resolution(&condition_id).await {
                Ok(r) => Some(FeedEvent::Resolution {
                    condition_id,
                    winner: r.map(|(winner, _)| winner),
                    resolved_at: r.map(|(_, at)| at),
                }),
                Err(e) => {
                    tracing::warn!(%slug, error = %e, "resolution fetch failed");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use raki_core::{Asset, StopKind, TradingMode};
    use raki_engine::engine::EngineConfig;
    use raki_feeds::rest::RestUrls;
    use sqlx::Row;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
    use tokio::sync::oneshot;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn drop_window_counts_growth_over_the_last_minute() {
        let t0 = 1_793_031_000_000;
        let mut w = DropWindow::new(t0, 0);
        assert_eq!(w.update(t0 + 1_000, 0), 0);
        assert_eq!(w.update(t0 + 2_000, 5), 5);
        assert_eq!(w.update(t0 + 30_000, 7), 7);
        // The jump to 5 happened at t0+2s; from t0+62s it is out of the window.
        assert_eq!(w.update(t0 + 61_000, 7), 7);
        assert_eq!(w.update(t0 + 62_000, 7), 2);
        assert_eq!(w.update(t0 + 91_000, 7), 0);
    }

    async fn postgres() -> (ContainerAsync<Postgres>, Store) {
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .expect("start postgres container (is Docker running?)");
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        (container, Store::connect(&url).await.unwrap())
    }

    async fn control(tx: &mpsc::Sender<ControlMsg>, cmd: ControlCommand) -> ControlResponse {
        let (reply, rx) = oneshot::channel();
        tx.send(ControlMsg { cmd, reply }).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap()
    }

    /// Polls `f` until it returns `Some` or 5 s pass.
    async fn eventually<T, F: Future<Output = Option<T>>>(mut f: impl FnMut() -> F) -> T {
        for _ in 0..100 {
            if let Some(v) = f().await {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition not met within 5 s");
    }

    async fn count(store: &Store, sql: &'static str) -> i64 {
        sqlx::query(sql)
            .fetch_one(&store.pool)
            .await
            .unwrap()
            .get::<i64, _>(0)
    }

    #[tokio::test]
    async fn engine_loop_runs_control_gaps_requests_and_live_snapshot() {
        let (_pg, store) = postgres().await;
        let gamma = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma)
            .await;
        let base = gamma.uri();
        let rest = PolyRest::new(
            RestUrls {
                gamma_base: base.clone(),
                data_base: base.clone(),
                geoblock_url: format!("{base}/api/geoblock"),
            },
            None,
        );

        let session = Session {
            id: Uuid::now_v7(),
            started_ms: now_ms(),
        };
        let mut engine = Engine::new(
            EngineConfig {
                assets: vec![Asset::Btc],
                session_id: session.id.to_string(),
            },
            now_ms(),
        );
        let (rec, _rec_rx) = raki_recorder::record_channel(16);
        let health = HealthInputs {
            feeds: FeedStatusRegistry::new(&[(Source::Binance, true)], HashMap::new()),
            state: HealthState::new(50.0),
            recorder: rec.stats(),
        };
        let initial = live_snapshot(&mut engine, &session, &health, 0, now_ms());
        assert_eq!(initial.control.stop_kind, Some(StopKind::Startup));

        let (events_tx, events_rx) = mpsc::channel(64);
        let (control_tx, control_rx) = mpsc::channel(4);
        let (desired_tx, _desired_rx) = watch::channel(BTreeSet::new());
        let (live_tx, mut live_rx) = watch::channel(initial);
        let shutdown = CancellationToken::new();
        let handle = tokio::spawn(run_engine_loop(
            engine,
            store.clone(),
            rest,
            session.clone(),
            LoopIo {
                events_tx: events_tx.clone(),
                events_rx,
                control_rx,
                desired_tx,
                live_tx,
            },
            health,
            shutdown.clone(),
        ));

        // The first tick publishes a snapshot and asks Gamma for the current and next window.
        tokio::time::timeout(Duration::from_secs(5), live_rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live_rx.borrow().session.id, session.id.to_string());
        assert_eq!(live_rx.borrow().windows[0].asset, Asset::Btc);
        eventually(|| async {
            let reqs = gamma.received_requests().await.unwrap_or_default();
            let slugs = reqs
                .iter()
                .filter(|r| r.url.path().starts_with("/events/slug/btc-updown-5m-"))
                .count();
            (slugs >= 2).then_some(())
        })
        .await;

        // Only a reconnect (every reset after the first per source) is a gap.
        for source in [Source::Binance, Source::Binance, Source::Coinbase] {
            events_tx
                .send(FeedEvent::FeedReset { source })
                .await
                .unwrap();
        }
        eventually(|| async {
            let n = count(
                &store,
                "select count(*) from feed_gaps where source = 'binance'",
            )
            .await;
            (n == 1).then_some(())
        })
        .await;
        assert_eq!(
            count(
                &store,
                "select count(*) from feed_gaps where source = 'coinbase'"
            )
            .await,
            0
        );

        let paused = control(
            &control_tx,
            ControlCommand::Pause {
                reason: Some("test".into()),
            },
        )
        .await;
        assert!(paused.ok);
        assert_eq!(paused.state.mode, TradingMode::Paused);
        assert_eq!(paused.state.stop_kind, Some(StopKind::Manual));

        // Clock and geoblock are unknown here, so resume is refused.
        let resumed = control(&control_tx, ControlCommand::Resume { acknowledge: true }).await;
        assert!(!resumed.ok);
        assert!(
            resumed
                .error
                .as_deref()
                .is_some_and(|e| e.contains("safety checks failing")),
            "{resumed:?}"
        );

        let killed = control(&control_tx, ControlCommand::Kill { reason: None }).await;
        assert!(killed.ok);
        assert_eq!(killed.state.mode, TradingMode::Killed);
        assert_eq!(live_rx.borrow().control.mode, TradingMode::Killed);

        let audit: Vec<(String, bool, String)> =
            sqlx::query_as("select action, ok, to_mode from control_audit order by id")
                .fetch_all(&store.pool)
                .await
                .unwrap();
        assert_eq!(
            audit,
            [
                ("pause".to_owned(), true, "paused".to_owned()),
                ("resume".to_owned(), false, "paused".to_owned()),
                ("kill".to_owned(), true, "killed".to_owned()),
            ]
        );

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .unwrap()
            .unwrap();
    }
}
