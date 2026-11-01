//! Store tests against a real Postgres 17 (testcontainers; Docker must be running).
//!
//! One container serves the whole file. Every test gets its own freshly created database so
//! counts and ordering never depend on test interleaving.

use std::sync::atomic::{AtomicU32, Ordering};

use raki_core::{
    Asset, Outcome, PredictionRow, SeriesRow, ShadowAction, ShadowEvalRow, ShadowFillRow,
    ShadowFillStatus, Source, TradingMode, WindowRecord, WindowStatus,
};
use raki_store::Store;
use rust_decimal::Decimal;
use sqlx::Row;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tokio::sync::OnceCell;
use uuid::Uuid;

/// Exact decimal from a literal; never goes through f64.
fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

struct Pg {
    // Held so the container lives for the whole test binary.
    _container: ContainerAsync<Postgres>,
    admin_url: String,
    base_url: String,
}

static PG: OnceCell<Pg> = OnceCell::const_new();
static DB_SEQ: AtomicU32 = AtomicU32::new(0);

async fn pg() -> &'static Pg {
    PG.get_or_init(|| async {
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .expect("start postgres container (is Docker running?)");
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let base_url = format!("postgres://postgres:postgres@127.0.0.1:{port}");
        Pg {
            admin_url: format!("{base_url}/postgres"),
            base_url,
            _container: container,
        }
    })
    .await
}

/// A store on a brand-new, migrated database.
async fn fresh_store() -> Store {
    let pg = pg().await;
    let name = format!(
        "t{}_{}",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::SeqCst)
    );
    let admin = Store::connect(&pg.admin_url).await.unwrap();
    // Test-only: `name` is generated here, never user input.
    sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
        .execute(&admin.pool)
        .await
        .unwrap();
    Store::connect(&format!("{}/{name}", pg.base_url))
        .await
        .unwrap()
}

fn window(slug: &str, asset: Asset, start_ts: i64, session: Uuid) -> WindowRecord {
    WindowRecord {
        slug: slug.into(),
        asset,
        start_ts,
        end_ts: start_ts + 300,
        event_id: Some("ev1".into()),
        condition_id: Some("0xcond".into()),
        up_token: Some("up".into()),
        down_token: Some("down".into()),
        price_to_beat_ours: Some(100.5),
        final_twap_ours: None,
        outcome_ours: None,
        price_to_beat_gamma: None,
        final_price_gamma: None,
        twap_match: None,
        winner: None,
        resolved_at: None,
        resolution_seen_ms: None,
        twap_source: None,
        final_twap_source: None,
        coverage_secs: 10,
        status: WindowStatus::Live,
        session_id: session.to_string(),
        updated_ms: 1_000,
    }
}

fn row(slug: &str, ts: i64, up_bid: Option<Decimal>) -> SeriesRow {
    SeriesRow {
        slug: slug.into(),
        ts,
        twap: Some(100.0 + ts as f64),
        chainlink: Some(1.0),
        binance_mid_usd: None,
        coinbase_mid: Some(2.5),
        up_bid,
        up_ask: Some(dec("0.5")),
        down_bid: None,
        down_ask: Some(dec("0.01")),
    }
}

#[tokio::test]
async fn migrations_apply() {
    let store = fresh_store().await;
    for q in [
        "select count(*) from sessions",
        "select count(*) from windows",
        "select count(*) from window_series",
        "select count(*) from feed_gaps",
        "select count(*) from control_audit",
    ] {
        let n: i64 = sqlx::query(q)
            .fetch_one(&store.pool)
            .await
            .unwrap_or_else(|e| panic!("{q}: {e}"))
            .get(0);
        assert_eq!(n, 0);
    }
    // Reconnecting re-runs migrate!() and must be a no-op.
    let again = Store::connect(&format!(
        "{}/{}",
        pg().await.base_url,
        sqlx::query("select current_database()")
            .fetch_one(&store.pool)
            .await
            .unwrap()
            .get::<String, _>(0)
    ))
    .await
    .unwrap();
    assert_eq!(again.recording_stats().await.unwrap().windows_total, 0);
}

#[tokio::test]
async fn upsert_window_roundtrip_and_update() {
    let store = fresh_store().await;
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    store
        .start_session(first, 1, "0.1.0", "host")
        .await
        .unwrap();
    store
        .start_session(second, 2, "0.1.0", "host")
        .await
        .unwrap();

    let mut w = window("btc-updown-5m-1000", Asset::Btc, 1000, first);
    store.upsert_window(&w).await.unwrap();
    assert_eq!(store.get_window(&w.slug).await.unwrap(), Some(w.clone()));

    // Later update from another session, with a lower coverage counter (restart mid-window).
    w.winner = Some(Outcome::Up);
    w.outcome_ours = Some(Outcome::Down);
    w.final_twap_ours = Some(101.25);
    w.twap_match = Some(false);
    w.twap_source = Some(Source::Polybolt);
    w.resolved_at = Some(1_300);
    w.resolution_seen_ms = Some(1_301_000);
    w.status = WindowStatus::Complete;
    w.updated_ms = 2_000;
    w.coverage_secs = 4;
    w.session_id = second.to_string();
    store.upsert_window(&w).await.unwrap();

    let got = store.get_window(&w.slug).await.unwrap().unwrap();
    assert_eq!(got.winner, Some(Outcome::Up));
    assert_eq!(got.outcome_ours, Some(Outcome::Down));
    assert_eq!(got.twap_source, Some(Source::Polybolt));
    assert_eq!(got.status, WindowStatus::Complete);
    assert_eq!(got.final_twap_ours, Some(101.25));
    assert_eq!(got.twap_match, Some(false));
    assert_eq!(got.session_id, first.to_string(), "first session is kept");
    assert_eq!(got.coverage_secs, 10, "max coverage is kept");

    // A higher counter wins.
    w.coverage_secs = 250;
    store.upsert_window(&w).await.unwrap();
    assert_eq!(
        store
            .get_window(&w.slug)
            .await
            .unwrap()
            .unwrap()
            .coverage_secs,
        250
    );

    assert_eq!(store.get_window("nope").await.unwrap(), None);
}

#[tokio::test]
async fn series_insert_idempotent() {
    let store = fresh_store().await;
    let rows = vec![
        row("s", 3, Some(dec("0.995"))),
        row("s", 1, Some(dec("0.005"))),
        row("s", 2, None),
    ];
    store.insert_series(&rows).await.unwrap();
    store.insert_series(&rows).await.unwrap();
    store.insert_series(&[]).await.unwrap();

    let got = store.get_series("s").await.unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got.iter().map(|r| r.ts).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(got[2].up_bid, Some(dec("0.995")));
    assert_eq!(got[0].up_bid, Some(dec("0.005")));
    assert_eq!(got[1].up_bid, None);
    assert_eq!(got[0], rows[1]);
    assert!(store.get_series("other").await.unwrap().is_empty());
}

#[tokio::test]
async fn list_windows_filters_and_orders() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1, "v", "h").await.unwrap();
    for (slug, asset, start) in [
        ("btc-a", Asset::Btc, 100),
        ("eth-a", Asset::Eth, 200),
        ("btc-b", Asset::Btc, 300),
        ("sol-a", Asset::Sol, 400),
        ("btc-c", Asset::Btc, 500),
    ] {
        store
            .upsert_window(&window(slug, asset, start, s))
            .await
            .unwrap();
    }

    let all = store.list_windows(None, 100, None).await.unwrap();
    assert_eq!(
        all.iter().map(|w| w.slug.as_str()).collect::<Vec<_>>(),
        ["btc-c", "sol-a", "btc-b", "eth-a", "btc-a"]
    );

    let btc = store
        .list_windows(Some(Asset::Btc), 100, None)
        .await
        .unwrap();
    assert_eq!(
        btc.iter().map(|w| w.slug.as_str()).collect::<Vec<_>>(),
        ["btc-c", "btc-b", "btc-a"]
    );

    let limited = store.list_windows(Some(Asset::Btc), 2, None).await.unwrap();
    assert_eq!(limited.len(), 2);

    // `before_start` is an exclusive cursor on start_ts.
    let page = store
        .list_windows(Some(Asset::Btc), 100, Some(500))
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|w| w.slug.as_str()).collect::<Vec<_>>(),
        ["btc-b", "btc-a"]
    );
    let page = store.list_windows(None, 1, Some(300)).await.unwrap();
    assert_eq!(page[0].slug, "eth-a");
}

#[tokio::test]
async fn recording_stats_counts() {
    let store = fresh_store().await;
    let empty = store.recording_stats().await.unwrap();
    assert_eq!(
        (empty.sessions, empty.recorded_secs, empty.windows_total),
        (0, 0, 0)
    );
    assert_eq!(empty.gate_target_windows, 500);

    let s1 = Uuid::now_v7();
    let s2 = Uuid::now_v7();
    store.start_session(s1, 1_000, "v", "h").await.unwrap();
    store.end_session(s1, 61_000, "shutdown").await.unwrap();
    store
        .start_session(s2, raki_core::now_ms(), "v", "h")
        .await
        .unwrap();
    // Awake 100 s .. 1000 s, except a sleep from 140 s to 160 s.
    let s3 = Uuid::now_v7();
    store.start_session(s3, 100_000, "v", "h").await.unwrap();
    store.record_sleep(s3, 160_000, 20_000).await.unwrap();
    store.end_session(s3, 1_000_000, "shutdown").await.unwrap();

    let no_final = |slug: &str, asset, end_ts: i64| {
        let mut w = window(slug, asset, end_ts - 300, s3);
        w.status = WindowStatus::Complete;
        w
    };
    for w in [
        // Awake through the end + 10 s: missed. ETH ended before PolyBolt began (end 400).
        no_final("m-eth", Asset::Eth, 30),
        no_final("m-btc", Asset::Btc, 500),
        // Not missed: asleep at the end; no session at the end; s1 ends before end + 10 s.
        no_final("m-asleep", Asset::Btc, 150),
        no_final("m-off", Asset::Btc, 70),
        no_final("m-late", Asset::Btc, 55),
    ] {
        store.upsert_window(&w).await.unwrap();
    }

    let with_final = |slug: &str, asset, start, final_source, matched: Option<bool>| {
        let mut w = window(slug, asset, start, s1);
        w.final_twap_ours = Some(1.0);
        w.final_twap_source = final_source;
        w.twap_match = matched;
        w
    };
    for w in [
        // Gate windows: BTC with a PolyBolt final point.
        with_final(
            "g-match",
            Asset::Btc,
            100,
            Some(Source::Polybolt),
            Some(true),
        ),
        with_final(
            "g-miss",
            Asset::Btc,
            110,
            Some(Source::Polybolt),
            Some(false),
        ),
        with_final("g-wait", Asset::Btc, 120, Some(Source::Polybolt), None),
        {
            let mut w = with_final("g-out", Asset::Btc, 130, Some(Source::Polybolt), None);
            w.status = WindowStatus::TimedOut;
            w
        },
        // Not gate windows: an RTDS final, another asset, an unknown final source.
        with_final(
            "rtds",
            Asset::Btc,
            140,
            Some(Source::RtdsLegacy),
            Some(true),
        ),
        with_final("eth", Asset::Eth, 150, Some(Source::Polybolt), Some(true)),
        with_final("old", Asset::Btc, 160, None, None),
        // No TWAP at all; and an upcoming window, which is not counted at all.
        window("n", Asset::Btc, 400, s1),
        {
            let mut w = window("u", Asset::Btc, 700, s1);
            w.status = WindowStatus::Upcoming;
            w
        },
    ] {
        store.upsert_window(&w).await.unwrap();
    }

    let st = store.recording_stats().await.unwrap();
    assert_eq!(st.sessions, 3);
    assert!(st.recorded_secs >= 60 + 880, "got {}", st.recorded_secs);
    assert_eq!(st.windows_total, 13, "upcoming excluded");
    assert_eq!(st.twap_missed_end, 2);
    assert_eq!(st.gate_twap_missed_end, 1, "BTC since the first PolyBolt final only");
    assert_eq!(st.windows_with_twap, 7);
    assert_eq!((st.twap_match, st.twap_mismatch), (3, 1));
    assert_eq!((st.twap_awaiting, st.twap_unverified), (2, 1));
    assert_eq!(st.gate_windows, 4, "BTC + PolyBolt final only");
    assert_eq!((st.gate_twap_match, st.gate_twap_mismatch), (1, 1));
    assert_eq!((st.gate_twap_awaiting, st.gate_twap_unverified), (1, 1));
    assert_eq!(st.gate_target_windows, 500);
}

#[tokio::test]
async fn upsert_never_regresses_known_fields_or_status() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1, "v", "h").await.unwrap();

    let mut full = window("btc-updown-5m-1000", Asset::Btc, 1000, s);
    full.price_to_beat_ours = Some(100.0);
    full.twap_source = Some(Source::Polybolt);
    full.final_twap_ours = Some(101.0);
    full.final_twap_source = Some(Source::Polybolt);
    full.outcome_ours = Some(Outcome::Up);
    full.price_to_beat_gamma = Some(100.0);
    full.winner = Some(Outcome::Up);
    full.resolved_at = Some(1_330);
    full.resolution_seen_ms = Some(1_326_000);
    full.status = WindowStatus::Resolved;
    full.updated_ms = 5_000;
    store.upsert_window(&full).await.unwrap();

    // A restarted engine's first record for the same window knows nothing yet.
    let mut sparse = window("btc-updown-5m-1000", Asset::Btc, 1000, Uuid::now_v7());
    sparse.event_id = None;
    sparse.condition_id = None;
    sparse.up_token = None;
    sparse.down_token = None;
    sparse.price_to_beat_ours = None;
    sparse.coverage_secs = 0;
    sparse.status = WindowStatus::Live;
    sparse.updated_ms = 6_000;
    store.upsert_window(&sparse).await.unwrap();

    let got = store.get_window(&full.slug).await.unwrap().unwrap();
    assert_eq!(
        got,
        WindowRecord {
            updated_ms: 6_000,
            ..full.clone()
        },
        "nothing regressed"
    );

    // A later, conflicting winner does not replace the latched one; new facts still land.
    let mut later = full.clone();
    later.winner = Some(Outcome::Down);
    later.resolved_at = Some(9_999);
    later.final_price_gamma = Some(101.0);
    later.twap_match = Some(true);
    later.status = WindowStatus::Complete;
    later.updated_ms = 7_000;
    store.upsert_window(&later).await.unwrap();
    let got = store.get_window(&full.slug).await.unwrap().unwrap();
    assert_eq!(got.winner, Some(Outcome::Up));
    assert_eq!(got.resolved_at, Some(1_330));
    assert_eq!(got.final_price_gamma, Some(101.0));
    assert_eq!(got.twap_match, Some(true));
    assert_eq!(got.status, WindowStatus::Complete);

    // Timed out ranks below complete, so a late final price still completes a window, and a
    // stale `resolved` cannot undo a timeout.
    let mut t = window("btc-updown-5m-2000", Asset::Btc, 2000, s);
    t.status = WindowStatus::TimedOut;
    store.upsert_window(&t).await.unwrap();
    t.status = WindowStatus::Resolved;
    store.upsert_window(&t).await.unwrap();
    assert_eq!(
        store.get_window(&t.slug).await.unwrap().unwrap().status,
        WindowStatus::TimedOut
    );
    t.status = WindowStatus::Complete;
    store.upsert_window(&t).await.unwrap();
    assert_eq!(
        store.get_window(&t.slug).await.unwrap().unwrap().status,
        WindowStatus::Complete
    );
}

#[tokio::test]
async fn dangling_sessions_close_as_crash_at_last_evidence() {
    let store = fresh_store().await;
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    // a: crashed; its heartbeat is the latest evidence.
    store.start_session(a, 1_000, "v", "h").await.unwrap();
    store.heartbeat(a, 50_000).await.unwrap();
    store
        .insert_gap(a, Source::Binance, 40_000, "reconnect")
        .await
        .unwrap();
    // b: crashed without a heartbeat; its window update is the evidence.
    store.start_session(b, 100_000, "v", "h").await.unwrap();
    let mut w = window("w", Asset::Btc, 100, b);
    w.updated_ms = 130_000;
    store.upsert_window(&w).await.unwrap();
    // c: ended cleanly.
    store.start_session(c, 200_000, "v", "h").await.unwrap();
    store.end_session(c, 260_000, "shutdown").await.unwrap();

    let mut closed = store.close_dangling_sessions(300_000).await.unwrap();
    closed.sort();
    let mut want = vec![(a, 50_000), (b, 130_000)];
    want.sort();
    assert_eq!(closed, want);
    assert!(
        store
            .close_dangling_sessions(300_000)
            .await
            .unwrap()
            .is_empty()
    );

    let reasons: Vec<(Uuid, Option<String>)> =
        sqlx::query_as("select id, end_reason from sessions order by started_ms")
            .fetch_all(&store.pool)
            .await
            .unwrap();
    assert_eq!(reasons[0].1.as_deref(), Some("crash"));
    assert_eq!(reasons[2].1.as_deref(), Some("shutdown"));
    let crash_gaps: i64 = sqlx::query(
        "select count(*) from feed_gaps where source = 'system' and reason like 'crash%'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(crash_gaps, 2);
    // Recorded time no longer runs on forever: 49 s + 30 s + 60 s.
    assert_eq!(store.recording_stats().await.unwrap().recorded_secs, 139);
}

#[tokio::test]
async fn sleep_is_a_gap_and_not_recorded_time() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 0, "v", "h").await.unwrap();
    store.record_sleep(s, 3_000_000, 1_800_000).await.unwrap();
    store.end_session(s, 3_600_000, "shutdown").await.unwrap();

    let gap = sqlx::query("select source, at_ms, reason, duration_ms from feed_gaps")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(gap.get::<String, _>("source"), "system");
    assert_eq!(gap.get::<String, _>("reason"), "sleep");
    assert_eq!(gap.get::<i64, _>("at_ms"), 1_200_000);
    assert_eq!(gap.get::<Option<i64>, _>("duration_ms"), Some(1_800_000));
    let st = store.recording_stats().await.unwrap();
    assert_eq!((st.recorded_secs, st.slept_secs), (1_800, 1_800));
}

#[tokio::test]
async fn unfinished_windows_for_adoption() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1, "v", "h").await.unwrap();
    for (slug, start, status) in [
        ("old", 100, WindowStatus::Ended),
        ("done", 1_000, WindowStatus::Complete),
        ("out", 1_300, WindowStatus::TimedOut),
        ("res", 1_600, WindowStatus::Resolved),
        ("live", 1_900, WindowStatus::Live),
    ] {
        let mut w = window(slug, Asset::Btc, start, s);
        w.status = status;
        store.upsert_window(&w).await.unwrap();
    }
    let got = store.unfinished_windows(1_000, 10).await.unwrap();
    assert_eq!(
        got.iter().map(|w| w.slug.as_str()).collect::<Vec<_>>(),
        ["live", "res", "out"]
    );
    assert_eq!(store.unfinished_windows(1_000, 1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn audit_and_gap_insert() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1, "v", "h").await.unwrap();

    store
        .insert_gap(s, Source::ClobMarket, 5_000, "ws closed")
        .await
        .unwrap();
    store
        .insert_audit(
            6_000,
            "pause",
            true,
            TradingMode::Running,
            TradingMode::Paused,
            Some("manual"),
            None,
        )
        .await
        .unwrap();
    store
        .insert_audit(
            7_000,
            "resume",
            false,
            TradingMode::Paused,
            TradingMode::Paused,
            None,
            Some("safety checks failing"),
        )
        .await
        .unwrap();

    let gap = sqlx::query("select source, at_ms, reason, session_id from feed_gaps")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(gap.get::<String, _>("source"), "clob_market");
    assert_eq!(gap.get::<i64, _>("at_ms"), 5_000);
    assert_eq!(gap.get::<String, _>("reason"), "ws closed");
    assert_eq!(gap.get::<Uuid, _>("session_id"), s);

    let audits = sqlx::query(
        "select action, ok, from_mode, to_mode, reason, error from control_audit order by id",
    )
    .fetch_all(&store.pool)
    .await
    .unwrap();
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[0].get::<String, _>("from_mode"), "running");
    assert_eq!(audits[0].get::<String, _>("to_mode"), "paused");
    assert_eq!(
        audits[0].get::<Option<String>, _>("reason").as_deref(),
        Some("manual")
    );
    assert!(audits[0].get::<bool, _>("ok"));
    assert!(!audits[1].get::<bool, _>("ok"));
    assert_eq!(audits[1].get::<Option<String>, _>("reason"), None);
    assert_eq!(
        audits[1].get::<Option<String>, _>("error").as_deref(),
        Some("safety checks failing")
    );
}

#[tokio::test]
async fn session_end_is_recorded() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1_000, "0.1.0", "box").await.unwrap();
    store.end_session(s, 9_000, "shutdown").await.unwrap();
    let r = sqlx::query("select ended_ms, end_reason, version, host from sessions where id = $1")
        .bind(s)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(r.get::<Option<i64>, _>("ended_ms"), Some(9_000));
    assert_eq!(
        r.get::<Option<String>, _>("end_reason").as_deref(),
        Some("shutdown")
    );
    assert_eq!(r.get::<String, _>("version"), "0.1.0");
    assert_eq!(r.get::<String, _>("host"), "box");
}

fn shadow_eval(session: Uuid, seq: i64, slug: &str, action: ShadowAction) -> ShadowEvalRow {
    ShadowEvalRow {
        session_id: session.to_string(),
        seq,
        strategy_id: "v0".into(),
        params_hash: "h1".into(),
        slug: slug.into(),
        ts_ms: 1_000 + seq,
        secs_left: 45,
        side: Some(Outcome::Up),
        model_prob: Some(0.98),
        market_prob: Some(dec("0.935")),
        best_ask: Some(dec("0.94")),
        walk_shares: Some(dec("5.3")),
        walk_avg_price: Some(dec("0.94")),
        walk_fee: Some(dec("0.021")),
        edge: Some(dec("0.036")),
        action,
        reason: (action == ShadowAction::Skip).then(|| "edge".into()),
        // The skip happened while control was paused.
        control_running: action == ShadowAction::Enter,
    }
}

fn shadow_fill(session: Uuid, seq: i64, slug: &str, side: Outcome, status: ShadowFillStatus) -> ShadowFillRow {
    ShadowFillRow {
        session_id: session.to_string(),
        seq,
        slug: slug.into(),
        side,
        decided_ms: 1_000,
        due_ms: 1_250,
        filled_ms: 1_300 + seq,
        limit_price: dec("0.94"),
        shares: dec("5"),
        cost: dec("4.7"),
        fee: dec("0.02"),
        status,
        winner: None,
        pnl: None,
    }
}

#[tokio::test]
async fn shadow_rows_summary_and_calibration() {
    let store = fresh_store().await;
    let s = Uuid::now_v7();
    store.start_session(s, 1, "v", "h").await.unwrap();
    // Two resolved windows (Up won both) and one still open.
    for (slug, start, winner) in [("w-up", 0, Some(Outcome::Up)), ("w-up2", 300, Some(Outcome::Up)), ("w-open", 600, None)] {
        let mut w = window(slug, Asset::Btc, start, s);
        w.winner = winner;
        w.status = if winner.is_some() { WindowStatus::Resolved } else { WindowStatus::Live };
        store.upsert_window(&w).await.unwrap();
    }
    store
        .upsert_strategy_params("h1", "v0", &serde_json::json!({"v0": {"min_edge": "0.01"}}), 5)
        .await
        .unwrap();

    let pred = |slug: &str, checkpoint_s, p_up| PredictionRow {
        slug: slug.into(),
        asset: Asset::Btc,
        checkpoint_s,
        ts_ms: 10,
        p_up,
        mu: 100.0,
        sd: 1.0,
        sigma: 0.1,
        ptb: 99.0,
        spot: 100.0,
        twap: Some(100.0),
        binance_mid_usd: None,
        coinbase_mid: None,
        up_bid: Some(dec("0.95")),
        up_ask: None,
        down_bid: None,
        down_ask: Some(dec("0.05")),
        model_id: "twap-gauss-v0".into(),
        session_id: s.to_string(),
    };
    for p in [pred("w-up", 30, 0.96), pred("w-up2", 30, 0.04), pred("w-up", 60, 0.7), pred("w-open", 30, 0.99)] {
        store.insert_prediction(&p).await.unwrap();
        store.insert_prediction(&p).await.unwrap(); // a retry is a no-op
    }
    let preds = store.window_predictions("w-up").await.unwrap();
    assert_eq!(preds.iter().map(|p| p.checkpoint_s).collect::<Vec<_>>(), [60, 30]);
    assert_eq!(preds[1], pred("w-up", 30, 0.96), "round trip");

    for e in [
        shadow_eval(s, 0, "w-up", ShadowAction::Skip),
        shadow_eval(s, 1, "w-up", ShadowAction::Enter),
        shadow_eval(s, 2, "w-up2", ShadowAction::Enter),
        shadow_eval(s, 3, "w-up2", ShadowAction::Enter),
        shadow_eval(s, 4, "w-open", ShadowAction::Enter),
    ] {
        store.insert_shadow_eval(&e).await.unwrap();
        store.insert_shadow_eval(&e).await.unwrap();
    }
    assert_eq!(store.window_shadow_evals("w-up").await.unwrap()[1], shadow_eval(s, 1, "w-up", ShadowAction::Enter));
    for f in [
        // Up won: +5 − 4.7 − 0.02 = 0.28.
        shadow_fill(s, 1, "w-up", Outcome::Up, ShadowFillStatus::Filled),
        // A no-fill, then a Down fill that lost: −4.72.
        shadow_fill(s, 2, "w-up2", Outcome::Down, ShadowFillStatus::NoFill),
        shadow_fill(s, 3, "w-up2", Outcome::Down, ShadowFillStatus::Partial),
        // Not resolved yet.
        shadow_fill(s, 4, "w-open", Outcome::Up, ShadowFillStatus::Filled),
    ] {
        store.insert_shadow_fill(&f).await.unwrap();
        store.insert_shadow_fill(&f).await.unwrap();
    }

    let fills = store.shadow_fills(Some("w-up"), 10).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!((fills[0].winner, fills[0].pnl), (Some(Outcome::Up), Some(dec("0.28"))));
    let all = store.shadow_fills(None, 10).await.unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].slug, "w-open", "newest first");
    assert_eq!(all[0].pnl, None);
    assert_eq!(all.iter().find(|f| f.seq == 2).unwrap().pnl, None, "a no-fill has no PnL");

    let sum = store.shadow_summary().await.unwrap();
    assert_eq!((sum.evaluations, sum.windows_evaluated, sum.windows_entered, sum.entries), (5, 3, 3, 4));
    assert_eq!(sum.evaluations_while_paused, 1);
    assert_eq!((sum.filled, sum.partial, sum.no_fill), (2, 1, 1));
    assert_eq!((sum.resolved, sum.wins, sum.losses), (2, 1, 1));
    assert_eq!(sum.staked, dec("9.44"));
    assert_eq!(sum.pnl, dec("-4.44"));
    assert!((sum.mean_predicted_edge.unwrap() - 0.036).abs() < 1e-12);
    assert_eq!(sum.skip_reasons.len(), 1);
    assert_eq!((sum.skip_reasons[0].reason.as_str(), sum.skip_reasons[0].count), ("edge", 1));
    assert_eq!(sum.params_hashes, ["h1"]);

    // At 30 s: w-up favoured Up at 0.96 (won), w-up2 favoured Down at 0.96 (lost); w-open has no
    // winner yet.
    let cal = store.calibration(30, Some(Asset::Btc)).await.unwrap();
    assert_eq!(cal.n, 2);
    let b = cal.buckets.iter().find(|b| b.label == "0.95–0.97").unwrap();
    assert_eq!((b.n, b.realized), (2, Some(0.5)));
    assert_eq!(store.calibration(30, Some(Asset::Eth)).await.unwrap().n, 0);
    assert_eq!(store.calibration(60, None).await.unwrap().n, 1);
}
