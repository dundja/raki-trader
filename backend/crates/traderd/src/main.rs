//! `traderd`: records the Polymarket 5-minute crypto markets and serves the dashboard API.
//!
//! M1 places no orders. Recording runs only while this process runs; every start is a new
//! session.

mod config;
mod health;
mod shell;
mod shutdown;
mod writer;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use raki_api::AppState;
use raki_core::{Source, now_ms, now_ns};
use raki_engine::engine::{Engine, EngineConfig};
use raki_feeds::rest::{PolyRest, RestUrls};
use raki_feeds::runners::{
    FeedCtx, PolyCreds, run_binance, run_clob, run_coinbase, run_polybolt, run_rtds_legacy,
};
use raki_feeds::status::FeedStatusRegistry;
use raki_recorder::{Record, RecordKind, RecordTx, spawn_recorder};
use raki_store::Store;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::{Config, Polybolt};
use crate::health::sleep::SleepSinks;
use crate::health::{HealthState, MaxAges};
use crate::shell::{HealthInputs, LoopIo, Session, StoreIo};
use crate::shutdown::{Stop, Tasks};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const CLOB_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const RTDS_URL: &str = "wss://ws-live-data.polymarket.com";
const POLYBOLT_URL: &str = "wss://ws-live-v2.polymarket.com/ws";
const BINANCE_BASE: &str = "wss://stream.binance.com:9443";
const COINBASE_URL: &str = "wss://ws-feed.exchange.coinbase.com";

const RECORDER_CAPACITY: usize = 200_000;
const EVENTS_CAPACITY: usize = 100_000;
const CONTROL_CAPACITY: usize = 64;
const SLEEP_NOTICE_CAPACITY: usize = 16;

/// At startup, unfinished windows that ended within this long are adopted and finished...
const ADOPT_WITHIN_S: i64 = 24 * 3600;
/// ...at most this many (newest first), so a big backlog cannot flood the APIs.
const ADOPT_MAX: i64 = 300;

/// Shutdown budget (5 s in all): tasks, then the session row, then the recorder's final frame.
const TASKS_STOP_WITHIN: Duration = Duration::from_millis(2_500);
const END_SESSION_WITHIN: Duration = Duration::from_millis(1_000);
const RECORDER_FINISH_WITHIN: Duration = Duration::from_millis(1_000);

#[derive(Parser)]
#[command(
    version,
    about = "Records Polymarket 5-minute crypto markets (M1: no trading)"
)]
struct Args {
    /// Path to traderd.toml.
    #[arg(long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // aws-lc-rs and ring are both linked; without an explicit default rustls panics.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    let cfg = config::load(&args.config)?;
    run(cfg).await
}

async fn run(cfg: Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("create data dir {}", cfg.data_dir.display()))?;
    // Bound before the session starts, so a busy port fails fast and leaves no session behind.
    let listener = tokio::net::TcpListener::bind(cfg.api_bind)
        .await
        .with_context(|| format!("bind api {}", cfg.api_bind))?;
    let store = Store::connect(&cfg.database_url).await?;

    // Sessions that never ended were cut short (crash, kill -9, power loss): close them at their
    // last sign of life, so recorded time stops growing, and log the crash as a gap.
    for (id, ended_ms) in store.close_dangling_sessions(now_ms()).await? {
        tracing::warn!(session = %id, ended_ms, "closed a session that ended without a clean shutdown");
    }
    let adopt = store
        .unfinished_windows(now_ms() / 1000 - ADOPT_WITHIN_S, ADOPT_MAX)
        .await?;

    let session = Session {
        id: Uuid::now_v7(),
        started_ms: now_ms(),
    };
    store
        .start_session(session.id, session.started_ms, VERSION, &hostname())
        .await?;
    let (rec, recorder) = spawn_recorder(cfg.data_dir.clone(), session.id, RECORDER_CAPACITY);
    let rec_stats = rec.stats();
    record_system(
        &rec,
        RecordKind::SessionStart,
        json!({ "session": session.id, "version": VERSION, "config": cfg.redacted_json() }),
    );
    tracing::info!(session = %session.id, version = VERSION, "session started");

    let creds = match cfg.feeds.polybolt {
        Polybolt::Off => None,
        Polybolt::Auto | Polybolt::On => PolyCreds::from_env(),
    };
    if cfg.feeds.polybolt == Polybolt::On && creds.is_none() {
        tracing::warn!("polybolt = true but the POLY_API_* env vars are not all set");
    }
    // Only whether it is on; the credential values are never logged.
    tracing::info!(enabled = creds.is_some(), "polybolt");
    let feeds = FeedStatusRegistry::new(
        &[
            (Source::ClobMarket, cfg.feeds.clob),
            (Source::RtdsLegacy, cfg.feeds.rtds_legacy),
            (Source::Polybolt, creds.is_some()),
            (Source::Binance, cfg.feeds.binance),
            (Source::Coinbase, cfg.feeds.coinbase),
        ],
        cfg.health.stale_ms.clone(),
    );
    // The CLOB runs one connection per asset: one socket for every token overflows the server's
    // per-connection send buffer at peak rates (close 1013 "slow consumer").
    if cfg.feeds.clob {
        feeds.expect_connections(Source::ClobMarket, cfg.assets.len() as u32);
    }
    let health = HealthState::new(cfg.health.clock_max_offset_ms);
    health.set_max_ages(MaxAges::from_intervals(
        Duration::from_secs(cfg.health.ntp_interval_s),
        Duration::from_secs(cfg.health.geoblock_interval_s),
    ));

    let (events_tx, events_rx) = mpsc::channel(EVENTS_CAPACITY);
    let (control_tx, control_rx) = mpsc::channel(CONTROL_CAPACITY);
    let (reconnect_tx, _) = broadcast::channel(4);
    let (sleep_tx, sleep_rx) = mpsc::channel(SLEEP_NOTICE_CAPACITY);
    let shutdown = CancellationToken::new();
    let mut tasks = Tasks::default();

    let ctx = || FeedCtx {
        rec: rec.clone(),
        events: events_tx.clone(),
        status: feeds.clone(),
        reconnect: reconnect_tx.subscribe(),
        shutdown: shutdown.clone(),
    };
    let assets = cfg.assets.clone();
    let mut desired_tx = BTreeMap::new();
    for &asset in &assets {
        let (tx, rx) = watch::channel(BTreeSet::new());
        desired_tx.insert(asset, tx);
        if cfg.feeds.clob {
            let name = match asset {
                raki_core::Asset::Btc => "clob-btc",
                raki_core::Asset::Eth => "clob-eth",
                raki_core::Asset::Sol => "clob-sol",
            };
            tasks.spawn(name, run_clob(ctx(), CLOB_URL.into(), Some(asset), rx));
        }
    }
    if cfg.feeds.rtds_legacy {
        tasks.spawn("rtds", run_rtds_legacy(ctx(), RTDS_URL.into()));
    }
    if let Some(creds) = creds {
        tasks.spawn(
            "polybolt",
            run_polybolt(ctx(), POLYBOLT_URL.into(), creds, assets.clone()),
        );
    }
    if cfg.feeds.binance {
        tasks.spawn(
            "binance",
            run_binance(ctx(), BINANCE_BASE.into(), assets.clone()),
        );
    }
    if cfg.feeds.coinbase {
        tasks.spawn(
            "coinbase",
            run_coinbase(ctx(), COINBASE_URL.into(), assets.clone()),
        );
    }

    let rest = || PolyRest::new(RestUrls::default(), Some(rec.clone()));
    tasks.spawn(
        "clock",
        health::clock::run_clock_monitor(
            health.clone(),
            cfg.health.ntp_servers.clone(),
            Duration::from_secs(cfg.health.ntp_interval_s),
            reconnect_tx.subscribe(),
            shutdown.clone(),
        ),
    );
    tasks.spawn(
        "sleep",
        health::sleep::run_sleep_monitor(
            SleepSinks {
                state: health.clone(),
                rec: rec.clone(),
                wake: reconnect_tx.clone(),
                notices: sleep_tx,
            },
            shutdown.clone(),
        ),
    );
    tasks.spawn(
        "geoblock",
        health::geoblock::run_geoblock_monitor(
            health.clone(),
            rest(),
            Duration::from_secs(cfg.health.geoblock_interval_s),
            reconnect_tx.subscribe(),
            shutdown.clone(),
        ),
    );

    let mut engine = Engine::new(
        EngineConfig {
            assets,
            session_id: session.id.to_string(),
        },
        now_ms(),
    );
    // Windows an earlier session left unfinished still get their winner, Gamma final price
    // and TWAP match.
    let adopted = adopt.len();
    for rec in adopt {
        engine.adopt(rec, now_ms());
    }
    if adopted > 0 {
        tracing::info!(
            windows = adopted,
            "adopted unfinished windows from earlier sessions"
        );
    }
    let (writer, writer_task) = writer::spawn_writer(store.clone(), session.id);
    let health_inputs = HealthInputs {
        feeds,
        state: health,
        recorder: rec_stats,
        store: writer.stats(),
    };
    let initial = shell::live_snapshot(&mut engine, &session, &health_inputs, 0, now_ms());
    let (live_tx, live_rx) = watch::channel(initial);

    let app = raki_api::router(
        AppState {
            store: store.clone(),
            live: live_rx,
            control: control_tx,
        },
        &cfg.cors_origin,
    );
    let server_stop = shutdown.clone();
    tasks.spawn("api", async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await;
        if let Err(e) = served {
            tracing::error!(error = %e, "api server failed");
        }
    });
    tasks.spawn(
        "engine",
        shell::run_engine_loop(
            engine,
            StoreIo {
                writer,
                task: writer_task,
            },
            rest(),
            session.clone(),
            LoopIo {
                events_tx,
                events_rx,
                control_rx,
                desired_tx,
                live_tx,
                sleep_rx,
            },
            health_inputs,
            shutdown.clone(),
        ),
    );
    tracing::info!(api = %cfg.api_bind, data_dir = %cfg.data_dir.display(), "traderd running");

    // Every task runs until shutdown. One that ends earlier (or the raw recorder hitting an I/O
    // error, e.g. a full disk) stops the whole process with a non-zero exit, so a recorder that
    // silently stopped recording never keeps running.
    let mut recorder = Some(recorder);
    let stop = shutdown::wait_for_stop(&mut tasks, &mut recorder, shutdown::signal()).await;
    match &stop {
        Stop::Signal(signal) => tracing::info!(signal, "shutting down"),
        Stop::Failed(what) => tracing::error!(%what, "a task stopped unexpectedly; shutting down"),
    }
    let reason = stop.reason();
    shutdown.cancel();
    shutdown::join_within(tasks.into_join_set(), TASKS_STOP_WITHIN).await;

    record_system(
        &rec,
        RecordKind::SessionEnd,
        json!({ "session": session.id, "reason": reason }),
    );
    match tokio::time::timeout(
        END_SESSION_WITHIN,
        store.end_session(session.id, now_ms(), &reason),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = ?e, "store: end session"),
        Err(_) => tracing::error!("store: end session timed out"),
    }
    // The last sender: the recorder finishes its zstd frame once it is gone.
    drop(rec);
    let finished = match recorder {
        None => Ok(()),
        Some(recorder) => match tokio::time::timeout(RECORDER_FINISH_WITHIN, recorder).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(e).context("recorder failed"),
            Ok(Err(e)) => Err(e).context("recorder task failed"),
            Err(_) => Err(anyhow::anyhow!(
                "recorder did not finish in time; the raw file is unfinished"
            )),
        },
    };
    match stop {
        Stop::Signal(_) => {
            finished?;
            tracing::info!(session = %session.id, "stopped");
            Ok(())
        }
        Stop::Failed(what) => {
            if let Err(e) = finished {
                tracing::error!(error = ?e, "recorder");
            }
            anyhow::bail!("stopped because a task failed: {what}")
        }
    }
}

fn record_system(rec: &RecordTx, k: RecordKind, d: serde_json::Value) {
    rec.send(Record {
        t: now_ns(),
        s: Source::System,
        c: 0,
        k,
        d: d.to_string(),
    });
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for `buf.len()` bytes for the duration of the call.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "unknown".to_owned();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}
