//! API tests: real Postgres 17 (testcontainers; Docker must be running), HTTP through
//! `tower::ServiceExt::oneshot`, and the WebSocket over a real bound listener.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures_util::StreamExt;
use raki_api::{AppState, ControlCommand, ControlMsg, router};
use raki_core::{
    Asset, ClockStatus, ControlResponse, ControlState, FeedStatusDto, GeoStatus, HealthSnapshot,
    LiveSnapshot, RecorderStatus, SafetyChecks, SeriesRow, SessionInfo, Source, StopKind,
    TradingMode, WindowRecord, WindowStatus,
};
use raki_store::Store;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tokio::sync::{OnceCell, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:3000";

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

/// A store on a brand-new, migrated database, so tests never see each other's rows.
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

fn control_state(mode: TradingMode) -> ControlState {
    ControlState {
        mode,
        stop_kind: Some(StopKind::Startup),
        reason: Some("startup".into()),
        needs_ack: false,
        since_ms: 1_000,
    }
}

fn snapshot(generated_ms: i64) -> LiveSnapshot {
    LiveSnapshot {
        generated_ms,
        session: SessionInfo {
            id: Uuid::nil().to_string(),
            started_ms: 500,
            version: "test".into(),
        },
        health: HealthSnapshot {
            feeds: vec![FeedStatusDto {
                source: Source::ClobMarket,
                enabled: true,
                connected: true,
                last_msg_ms: Some(generated_ms),
                msgs_total: 7,
                msgs_per_sec: 1.5,
                reconnects: 0,
                last_error: None,
                stale: false,
            }],
            clock: ClockStatus {
                offset_ms: Some(1.0),
                ok: true,
                checked_ms: Some(generated_ms),
                max_offset_ms: 50.0,
            },
            geoblock: GeoStatus {
                blocked: Some(false),
                country: Some("RS".into()),
                checked_ms: Some(generated_ms),
            },
            recorder: RecorderStatus {
                file: None,
                records: 3,
                bytes: 99,
                dropped: 0,
                last_flush_ms: None,
            },
            sleep_events: 0,
            book_resyncs: 0,
            safety: SafetyChecks {
                feeds_fresh: true,
                clock_ok: true,
                geoblock_ok: true,
                recorder_ok: true,
                all_ok: true,
                reasons: vec![],
            },
        },
        control: control_state(TradingMode::Paused),
        windows: vec![],
    }
}

fn window(slug: &str, asset: Asset, start_ts: i64) -> WindowRecord {
    WindowRecord {
        slug: slug.into(),
        asset,
        start_ts,
        end_ts: start_ts + 300,
        event_id: None,
        condition_id: None,
        up_token: None,
        down_token: None,
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
        coverage_secs: 10,
        status: WindowStatus::Live,
        session_id: Uuid::nil().to_string(),
        updated_ms: 1_000,
    }
}

struct Harness {
    app: Router,
    store: Store,
    live_tx: watch::Sender<LiveSnapshot>,
    control_rx: mpsc::Receiver<ControlMsg>,
}

async fn harness() -> Harness {
    let store = fresh_store().await;
    let (live_tx, live) = watch::channel(snapshot(1_000));
    let (control, control_rx) = mpsc::channel(8);
    let app = router(
        AppState {
            store: store.clone(),
            live,
            control,
        },
        ORIGIN,
    );
    Harness {
        app,
        store,
        live_tx,
        control_rx,
    }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, body.to_vec())
}

async fn get_json<T: DeserializeOwned>(app: &Router, uri: &str) -> (StatusCode, T) {
    let req = Request::get(uri).body(Body::empty()).unwrap();
    let (status, body) = send(app, req).await;
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("{uri}: {e}: {}", String::from_utf8_lossy(&body)));
    (status, value)
}

fn post(uri: &str, body: Option<&str>) -> Request<Body> {
    let mut b = Request::post(uri);
    let body = match body {
        Some(json) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_owned())
        }
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

#[tokio::test]
async fn live_returns_snapshot() {
    let h = harness().await;
    let (status, snap): (_, LiveSnapshot) = get_json(&h.app, "/api/live").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snap, snapshot(1_000));

    h.live_tx.send(snapshot(2_000)).unwrap();
    let (_, snap): (_, LiveSnapshot) = get_json(&h.app, "/api/live").await;
    assert_eq!(snap.generated_ms, 2_000);

    let (status, health): (_, HealthSnapshot) = get_json(&h.app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health, snapshot(2_000).health);
}

#[tokio::test]
async fn windows_list_and_get_and_404() {
    let h = harness().await;
    for (slug, asset, start) in [
        ("btc-updown-5m-300", Asset::Btc, 300),
        ("btc-updown-5m-600", Asset::Btc, 600),
        ("btc-updown-5m-900", Asset::Btc, 900),
        ("eth-updown-5m-600", Asset::Eth, 600),
    ] {
        h.store
            .upsert_window(&window(slug, asset, start))
            .await
            .unwrap();
    }

    let (status, all): (_, Vec<WindowRecord>) = get_json(&h.app, "/api/windows").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all.len(), 4);

    let (_, btc): (_, Vec<WindowRecord>) = get_json(&h.app, "/api/windows?asset=btc").await;
    let starts: Vec<i64> = btc.iter().map(|w| w.start_ts).collect();
    assert_eq!(starts, [900, 600, 300], "newest first, btc only");

    let (_, page): (_, Vec<WindowRecord>) =
        get_json(&h.app, "/api/windows?asset=btc&limit=1&before=900").await;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].slug, "btc-updown-5m-600");

    // limit is clamped to at least 1 rather than rejected.
    let (status, clamped): (_, Vec<WindowRecord>) = get_json(&h.app, "/api/windows?limit=0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(clamped.len(), 1);

    let (status, one): (_, WindowRecord) = get_json(&h.app, "/api/windows/btc-updown-5m-600").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one, window("btc-updown-5m-600", Asset::Btc, 600));

    let (status, err): (_, serde_json::Value) =
        get_json(&h.app, "/api/windows/btc-updown-5m-1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(err["error"].as_str().is_some_and(|e| !e.is_empty()));
}

#[tokio::test]
async fn windows_bad_asset_400() {
    let h = harness().await;
    let (status, err): (_, serde_json::Value) = get_json(&h.app, "/api/windows?asset=doge").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("doge"));

    let (status, err): (_, serde_json::Value) = get_json(&h.app, "/api/windows?limit=lots").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].is_string());
}

#[tokio::test]
async fn series_endpoint_returns_rows_in_order() {
    let h = harness().await;
    let slug = "btc-updown-5m-300";
    let row = |ts: i64, up_bid: Option<Decimal>| SeriesRow {
        slug: slug.into(),
        ts,
        twap: Some(100.0 + ts as f64),
        chainlink: None,
        binance_mid_usd: None,
        coinbase_mid: None,
        up_bid,
        up_ask: None,
        down_bid: None,
        down_ask: None,
    };
    // Inserted out of order on purpose.
    h.store
        .insert_series(&[
            row(302, None),
            row(300, Some("0.52".parse().unwrap())),
            row(301, None),
        ])
        .await
        .unwrap();

    let (status, rows): (_, Vec<SeriesRow>) =
        get_json(&h.app, &format!("/api/windows/{slug}/series")).await;
    assert_eq!(status, StatusCode::OK);
    let ts: Vec<i64> = rows.iter().map(|r| r.ts).collect();
    assert_eq!(ts, [300, 301, 302]);
    assert_eq!(rows[0].up_bid, Some("0.52".parse().unwrap()));

    let (status, rows): (_, Vec<SeriesRow>) =
        get_json(&h.app, "/api/windows/unknown-slug/series").await;
    assert_eq!(status, StatusCode::OK);
    assert!(rows.is_empty());
}

#[tokio::test]
async fn stats_endpoint() {
    let h = harness().await;
    h.store
        .upsert_window(&window("btc-updown-5m-300", Asset::Btc, 300))
        .await
        .unwrap();
    let (status, stats): (_, raki_core::RecordingStats) = get_json(&h.app, "/api/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats.windows_total, 1);
    assert_eq!(stats.gate_target_windows, 500);
}

/// Answers exactly one control message and returns the command it saw.
fn answer_once(
    mut rx: mpsc::Receiver<ControlMsg>,
    resp: ControlResponse,
) -> tokio::task::JoinHandle<ControlCommand> {
    tokio::spawn(async move {
        let msg = rx.recv().await.expect("a control message");
        msg.reply.send(resp).unwrap();
        msg.cmd
    })
}

#[tokio::test]
async fn control_resume_roundtrip() {
    let h = harness().await;
    let responder = answer_once(
        h.control_rx,
        ControlResponse {
            ok: false,
            state: control_state(TradingMode::Paused),
            error: Some("safety checks failing: x".into()),
        },
    );

    let (status, body) = send(
        &h.app,
        post(
            "/api/control/resume",
            Some(r#"{"reason":null,"acknowledge":true}"#),
        ),
    )
    .await;
    // A refusal is a normal answer: 200 with ok == false.
    assert_eq!(status, StatusCode::OK);
    let resp: ControlResponse = serde_json::from_slice(&body).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_deref(), Some("safety checks failing: x"));
    assert_eq!(resp.state.mode, TradingMode::Paused);
    assert_eq!(
        responder.await.unwrap(),
        ControlCommand::Resume { acknowledge: true }
    );
}

/// A router wired to a fresh control channel, so each case gets its own responder.
fn app_with_control(h: &Harness) -> (Router, mpsc::Receiver<ControlMsg>) {
    let (control, rx) = mpsc::channel(8);
    let app = router(
        AppState {
            store: h.store.clone(),
            live: h.live_tx.subscribe(),
            control,
        },
        ORIGIN,
    );
    (app, rx)
}

#[tokio::test]
async fn control_pause_and_kill_forward_reason_with_optional_body() {
    let h = harness().await;
    let ok = || ControlResponse {
        ok: true,
        state: control_state(TradingMode::Paused),
        error: None,
    };

    // Pause with a reason.
    let (app, rx) = app_with_control(&h);
    let responder = answer_once(rx, ok());
    let (status, _) = send(
        &app,
        post(
            "/api/control/pause",
            Some(r#"{"reason":"lunch","acknowledge":null}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        responder.await.unwrap(),
        ControlCommand::Pause {
            reason: Some("lunch".into())
        }
    );

    // Kill with no body at all.
    let (app, rx) = app_with_control(&h);
    let responder = answer_once(rx, ok());
    let (status, _) = send(&app, post("/api/control/kill", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        responder.await.unwrap(),
        ControlCommand::Kill { reason: None }
    );

    // A malformed body is the caller's mistake.
    let (status, _) = send(&app, post("/api/control/kill", Some("{nope"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn control_timeout_503() {
    let h = harness().await;
    // `control_rx` stays alive but nobody answers.
    let started = tokio::time::Instant::now();
    let (status, body) = send(&h.app, post("/api/control/pause", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].is_string());
    assert!(started.elapsed() >= Duration::from_secs(2));
    drop(h.control_rx);
}

#[tokio::test]
async fn control_503_when_handler_is_gone() {
    let h = harness().await;
    drop(h.control_rx);
    let (status, _) = send(&h.app, post("/api/control/pause", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

async fn next_snapshot<S>(ws: &mut S) -> LiveSnapshot
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("a frame within 5 s")
        .expect("stream open")
        .expect("no ws error");
    match msg {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

#[tokio::test]
async fn ws_pushes_initial_and_updates() {
    let h = harness().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, h.app).await.unwrap() });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws"))
        .await
        .unwrap();
    let first = next_snapshot(&mut ws).await;
    assert_eq!(first.generated_ms, 1_000);

    h.live_tx.send(snapshot(2_000)).unwrap();
    let second = next_snapshot(&mut ws).await;
    assert_eq!(second.generated_ms, 2_000);
    assert_eq!(second, snapshot(2_000));
}

#[tokio::test]
async fn cors_allows_dashboard_origin() {
    let h = harness().await;
    let req = Request::options("/api/control/pause")
        .header(header::ORIGIN, ORIGIN)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert_eq!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        ORIGIN
    );

    // A different origin is not echoed back.
    let req = Request::options("/api/control/pause")
        .header(header::ORIGIN, "http://evil.example")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_ne!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|v| v.to_str().unwrap()),
        Some("http://evil.example")
    );
}
