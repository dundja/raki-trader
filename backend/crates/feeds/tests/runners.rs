//! Runner tests against local scripted WebSocket servers (`ws://127.0.0.1:<port>`), in real time
//! with short timeouts and a fast backoff.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use raki_core::{Asset, FeedEvent, Source};
use raki_feeds::backoff::Backoff;
use raki_feeds::runners::{
    BinanceTiming, ClobTiming, CoinbaseTiming, FeedCtx, PolyCreds, PolyboltTiming, RtdsTiming,
    run_binance, run_binance_with, run_clob, run_clob_with, run_coinbase, run_coinbase_with,
    run_polybolt_with, run_rtds_legacy, run_rtds_legacy_with,
};
use raki_feeds::status::FeedStatusRegistry;
use raki_recorder::{Record, RecordKind, record_channel};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(2);
const SOURCES: [Source; 5] = [
    Source::ClobMarket,
    Source::RtdsLegacy,
    Source::Polybolt,
    Source::Binance,
    Source::Coinbase,
];

type Ws = WebSocketStream<TcpStream>;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Installs the process-level rustls provider, as `main` does. Idempotent.
fn ensure_crypto() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

fn fast_backoff() -> Backoff {
    Backoff::new(ms(10), ms(50))
}

/// The test's side of a [`FeedCtx`].
struct Harness {
    rec: mpsc::Receiver<Record>,
    events: mpsc::Receiver<FeedEvent>,
    status: Arc<FeedStatusRegistry>,
    reconnect: broadcast::Sender<()>,
    shutdown: CancellationToken,
}

fn harness() -> (FeedCtx, Harness) {
    ensure_crypto();
    let (rec_tx, rec_rx) = record_channel(4096);
    let (ev_tx, ev_rx) = mpsc::channel(4096);
    let enabled = SOURCES.map(|s| (s, true));
    let status = FeedStatusRegistry::new(&enabled, SOURCES.map(|s| (s, 1_000)).into());
    let (reconnect_tx, reconnect_rx) = broadcast::channel(4);
    let shutdown = CancellationToken::new();
    let ctx = FeedCtx {
        rec: rec_tx,
        events: ev_tx,
        status: status.clone(),
        reconnect: reconnect_rx,
        shutdown: shutdown.clone(),
    };
    let h = Harness {
        rec: rec_rx,
        events: ev_rx,
        status,
        reconnect: reconnect_tx,
        shutdown,
    };
    (ctx, h)
}

impl Harness {
    async fn next_record(&mut self) -> Record {
        timeout(WAIT, self.rec.recv())
            .await
            .expect("no record in time")
            .expect("record channel closed")
    }

    /// The next record of `kind`, skipping others.
    async fn record_of(&mut self, kind: RecordKind) -> Record {
        loop {
            let r = self.next_record().await;
            if r.k == kind {
                return r;
            }
        }
    }

    async fn next_event(&mut self) -> FeedEvent {
        timeout(WAIT, self.events.recv())
            .await
            .expect("no event in time")
            .expect("event channel closed")
    }

    /// Cancels the runner and checks that it returns promptly.
    async fn stop(&self, runner: JoinHandle<()>) {
        self.shutdown.cancel();
        timeout(WAIT, runner)
            .await
            .expect("runner did not stop on shutdown")
            .expect("runner panicked");
    }
}

struct Server {
    listener: TcpListener,
    url: String,
}

async fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    Server { listener, url }
}

impl Server {
    async fn tcp(&self) -> TcpStream {
        let (tcp, _) = timeout(WAIT, self.listener.accept())
            .await
            .expect("client did not connect in time")
            .unwrap();
        tcp
    }

    async fn accept(&self) -> Ws {
        tokio_tungstenite::accept_async(self.tcp().await)
            .await
            .unwrap()
    }

    /// Accepts and returns the request path with its query string.
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite fixes the handshake callback's error type"
    )]
    async fn accept_with_path(&self) -> (Ws, String) {
        let path = Arc::new(Mutex::new(String::new()));
        let seen = path.clone();
        let ws = tokio_tungstenite::accept_hdr_async(
            self.tcp().await,
            move |req: &Request, resp: Response| {
                *seen.lock().unwrap() = req.uri().to_string();
                Ok(resp)
            },
        )
        .await
        .unwrap();
        let path = path.lock().unwrap().clone();
        (ws, path)
    }

    /// Asserts that no client connects within `d`.
    async fn assert_no_connect_within(&self, d: Duration) {
        assert!(
            timeout(d, self.listener.accept()).await.is_err(),
            "unexpected reconnect within {d:?}"
        );
    }
}

/// The next text frame the client sent, skipping control frames.
async fn next_text(ws: &mut Ws) -> String {
    timeout(WAIT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => return t.to_string(),
                Some(Ok(_)) => continue,
                other => panic!("client stream ended: {other:?}"),
            }
        }
    })
    .await
    .expect("no text frame from the client in time")
}

async fn next_json(ws: &mut Ws) -> Value {
    serde_json::from_str(&next_text(ws).await).unwrap()
}

async fn send_text(ws: &mut Ws, text: &str) {
    ws.send(Message::text(text)).await.unwrap();
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn ids(v: &Value) -> BTreeSet<String> {
    v["assets_ids"]
        .as_array()
        .expect("assets_ids array")
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

fn spawn_clob(
    ctx: FeedCtx,
    url: &str,
    desired: &[&str],
    timing: ClobTiming,
) -> (watch::Sender<BTreeSet<String>>, JoinHandle<()>) {
    spawn_clob_with(ctx, url, desired, fast_backoff(), timing)
}

fn spawn_clob_with(
    ctx: FeedCtx,
    url: &str,
    desired: &[&str],
    backoff: Backoff,
    timing: ClobTiming,
) -> (watch::Sender<BTreeSet<String>>, JoinHandle<()>) {
    let (tx, rx) = watch::channel(set(desired));
    let runner = tokio::spawn(run_clob_with(ctx, url.to_owned(), rx, backoff, timing));
    (tx, runner)
}

const BOOK: &str = r#"{"event_type":"book","asset_id":"a","market":"0xm","timestamp":"1793031216412","bids":[{"price":"0.48","size":"10"}],"asks":[{"price":"0.52","size":"5"}],"hash":"h"}"#;

#[tokio::test]
async fn clob_sends_initial_subscribe_with_custom_features() {
    let (ctx, h) = harness();
    let srv = server().await;
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &["a", "b"], ClobTiming::default());

    let mut ws = srv.accept().await;
    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "market");
    assert_eq!(first["custom_feature_enabled"], true);
    assert_eq!(ids(&first), set(&["a", "b"]));

    h.stop(runner).await;
}

#[tokio::test]
async fn clob_dynamic_subscribe_and_unsubscribe() {
    let (ctx, h) = harness();
    let srv = server().await;
    let (desired, runner) = spawn_clob(ctx, &srv.url, &["a", "b"], ClobTiming::default());

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    desired.send(set(&["b", "c"])).unwrap();

    let frames = [next_json(&mut ws).await, next_json(&mut ws).await];
    let by_op = |op: &str| {
        frames
            .iter()
            .find(|f| f["operation"] == op)
            .unwrap_or_else(|| panic!("no {op} frame in {frames:?}"))
    };
    assert_eq!(ids(by_op("subscribe")), set(&["c"]));
    assert_eq!(ids(by_op("unsubscribe")), set(&["a"]));

    h.stop(runner).await;
}

#[tokio::test]
async fn clob_stays_connected_without_subscribing_when_desired_is_empty() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let timing = ClobTiming {
        ping: ms(50),
        pong_timeout: ms(100),
        ..ClobTiming::default()
    };
    let (desired, runner) = spawn_clob(ctx, &srv.url, &[], timing);

    let mut ws = srv.accept().await;
    // The live server closes with 1008 on a PING before any subscribe, so the heartbeat (and
    // its PONG deadline) waits for the first subscribe. Nothing is sent and nothing drops.
    assert!(
        timeout(ms(300), ws.next()).await.is_err(),
        "the client sent a frame or disconnected while unsubscribed"
    );
    desired.send(set(&["x"])).unwrap();
    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "market");
    assert_eq!(first["custom_feature_enabled"], true);
    assert_eq!(ids(&first), set(&["x"]));
    assert_eq!(next_text(&mut ws).await, "PING");
    assert_eq!(h.record_of(RecordKind::Open).await.d, srv.url);

    h.stop(runner).await;
}

#[tokio::test]
async fn clob_reconnects_and_resubscribes_after_server_close() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let timing = ClobTiming {
        clean_close_delay: ms(20),
        ..ClobTiming::default()
    };
    // A slow backoff: only the clean-close fast path reconnects within the deadline below.
    let backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(5));
    let (_desired, runner) = spawn_clob_with(ctx, &srv.url, &["a", "b"], backoff, timing);

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    send_text(&mut ws, BOOK).await;
    ws.close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    }))
    .await
    .unwrap();

    let (tcp, _) = timeout(ms(500), srv.listener.accept())
        .await
        .expect("no fast reconnect after a clean close")
        .unwrap();
    let mut ws2 = tokio_tungstenite::accept_async(tcp).await.unwrap();
    let again = next_json(&mut ws2).await;
    assert_eq!(again["type"], "market");
    assert_eq!(again["custom_feature_enabled"], true);
    assert_eq!(ids(&again), set(&["a", "b"]));

    let mut recs = Vec::new();
    for _ in 0..6 {
        recs.push(h.next_record().await);
    }
    let kinds: Vec<RecordKind> = recs.iter().map(|r| r.k).collect();
    use RecordKind::*;
    assert_eq!(kinds, [Open, Send, Text, Close, Gap, Open]);
    assert_eq!(recs[2].d, BOOK);
    assert!(recs[3].d.contains("1000"), "close record: {}", recs[3].d);
    let gap: Value = serde_json::from_str(&recs[4].d).unwrap();
    assert_eq!(gap["source"], "clob_market");
    assert!(gap["reason"].is_string());
    assert!(recs.iter().all(|r| r.s == Source::ClobMarket));
    assert!(recs[5].c > recs[0].c, "connection number increases");

    let reset = FeedEvent::FeedReset {
        source: Source::ClobMarket,
    };
    assert_eq!(h.next_event().await, reset);
    assert!(matches!(h.next_event().await, FeedEvent::Book { .. }));
    assert_eq!(h.next_event().await, reset);

    h.stop(runner).await;
}

#[tokio::test]
async fn clob_sends_ping_and_accepts_pong() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let timing = ClobTiming {
        ping: ms(50),
        pong_timeout: ms(150),
        ..ClobTiming::default()
    };
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &["a"], timing);

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    let pings = Arc::new(Mutex::new(0));
    let counted = pings.clone();
    let responder = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws.next().await {
            if msg == Message::text("PING") {
                *counted.lock().unwrap() += 1;
                ws.send(Message::text("PONG")).await.unwrap();
            }
        }
    });

    srv.assert_no_connect_within(ms(300)).await;
    assert!(
        *pings.lock().unwrap() >= 3,
        "pings: {}",
        pings.lock().unwrap()
    );
    let subscribe = h.record_of(RecordKind::Send).await;
    assert!(subscribe.d.contains("assets_ids"));
    assert_eq!(h.record_of(RecordKind::Send).await.d, "PING");
    assert_eq!(h.record_of(RecordKind::Text).await.d, "PONG");

    h.stop(runner).await;
    responder.abort();
}

#[tokio::test]
async fn clob_reconnects_when_pong_is_missing() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let timing = ClobTiming {
        ping: ms(50),
        pong_timeout: ms(150),
        ..ClobTiming::default()
    };
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &["a"], timing);

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    // Read (so the client's writes succeed) but never answer.
    let silent = tokio::spawn(async move { while ws.next().await.is_some() {} });

    let mut ws2 = srv.accept().await;
    assert_eq!(ids(&next_json(&mut ws2).await), set(&["a"]));
    let err = h.record_of(RecordKind::Error).await;
    assert!(err.d.contains("PONG"), "error record: {}", err.d);
    h.record_of(RecordKind::Gap).await;

    h.stop(runner).await;
    silent.abort();
}

#[tokio::test]
async fn raw_text_recorded_before_parse_even_if_garbage() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &[], ClobTiming::default());

    let mut ws = srv.accept().await;
    send_text(&mut ws, "{not json").await;

    let text = h.record_of(RecordKind::Text).await;
    assert_eq!(text.d, "{not json");
    assert_eq!(text.s, Source::ClobMarket);
    assert!(text.t > 1_700_000_000_000_000_000, "t is epoch ns");
    assert_eq!(
        h.next_event().await,
        FeedEvent::FeedReset {
            source: Source::ClobMarket
        }
    );
    assert!(
        timeout(ms(100), h.events.recv()).await.is_err(),
        "garbage must not produce an event"
    );
    // The runner survives the parse error.
    send_text(&mut ws, BOOK).await;
    assert!(matches!(h.next_event().await, FeedEvent::Book { .. }));

    h.stop(runner).await;
}

#[tokio::test]
async fn reconnect_broadcast_forces_reconnect() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &["a", "b"], ClobTiming::default());

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    h.reconnect.send(()).unwrap();

    let mut ws2 = srv.accept().await;
    assert_eq!(ids(&next_json(&mut ws2).await), set(&["a", "b"]));
    h.record_of(RecordKind::Close).await;
    h.record_of(RecordKind::Gap).await;
    h.record_of(RecordKind::Open).await;

    h.stop(runner).await;
}

fn creds() -> PolyCreds {
    PolyCreds {
        api_key: SecretString::from("k"),
        secret: SecretString::from("s"),
        passphrase: SecretString::from("p"),
    }
}

#[tokio::test]
async fn polybolt_auth_redacted_in_record_and_subscribes_after_authed() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_polybolt_with(
        ctx,
        srv.url.clone(),
        creds(),
        vec![Asset::Btc, Asset::Eth],
        fast_backoff(),
        PolyboltTiming::default(),
    ));

    let mut ws = srv.accept().await;
    let auth_text = next_text(&mut ws).await;
    assert!(auth_text.contains(r#""apiKey":"k""#), "{auth_text}");
    let auth: Value = serde_json::from_str(&auth_text).unwrap();
    assert_eq!(auth["op"], "auth");
    assert_eq!(auth["rid"], "a1");
    assert_eq!(
        auth["auth"],
        json!({"apiKey": "k", "secret": "s", "passphrase": "p"})
    );

    let sent = h.record_of(RecordKind::Send).await;
    assert_eq!(sent.d, r#"{"op":"auth","rid":"a1","auth":"<redacted>"}"#);
    assert!(!sent.d.contains("\"secret\""));

    // Nothing is subscribed before `authed`.
    assert!(timeout(ms(100), ws.next()).await.is_err());
    send_text(&mut ws, r#"{"op":"authed","rid":"a1"}"#).await;

    let sub = next_json(&mut ws).await;
    assert_eq!(sub["op"], "subscribe");
    let subs = sub["subscriptions"].as_array().unwrap();
    assert_eq!(subs.len(), 4, "2 subscriptions per asset: {sub}");
    for asset in ["btcusd", "ethusd"] {
        assert!(subs.contains(&json!({"channel": "price.crypto", "filter": {"symbol": asset}})));
        assert!(subs.contains(&json!({
            "channel": "price.crypto.twap",
            "filter": {"symbol": asset, "window_seconds": 60}
        })));
    }

    h.stop(runner).await;
}

#[tokio::test]
async fn polybolt_auth_invalid_backs_off_at_max() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_polybolt_with(
        ctx,
        srv.url.clone(),
        creds(),
        vec![Asset::Btc],
        Backoff::new(ms(10), ms(400)),
        PolyboltTiming::default(),
    ));

    let mut ws = srv.accept().await;
    next_text(&mut ws).await;
    send_text(
        &mut ws,
        r#"{"op":"error","rid":"a1","code":"auth_invalid"}"#,
    )
    .await;

    assert!(
        h.record_of(RecordKind::Error)
            .await
            .d
            .contains("auth_invalid")
    );
    h.record_of(RecordKind::Gap).await;
    let snap = h.status.snapshot(raki_core::now_ms());
    let pb = snap.iter().find(|f| f.source == Source::Polybolt).unwrap();
    assert_eq!(pb.last_error.as_deref(), Some("auth_invalid"));
    assert!(!pb.connected);

    srv.assert_no_connect_within(ms(250)).await;
    srv.accept().await;

    h.stop(runner).await;
}

#[tokio::test]
async fn polybolt_seq_gap_is_recorded() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_polybolt_with(
        ctx,
        srv.url.clone(),
        creds(),
        vec![Asset::Btc],
        fast_backoff(),
        PolyboltTiming::default(),
    ));

    let mut ws = srv.accept().await;
    next_text(&mut ws).await;
    send_text(&mut ws, r#"{"op":"authed","rid":"a1"}"#).await;
    next_json(&mut ws).await;
    let twap = |seq: u64| {
        format!(
            r#"{{"v":1,"channel":"price.crypto.twap","seq":{seq},"ts":1793031000500,"payload":{{"symbol":"btcusd","value":80000.5,"full_accuracy_value":"80000.5","timestamp":1793031000000,"window_seconds":60,"source":"chainlink"}}}}"#
        )
    };
    send_text(&mut ws, &twap(1)).await;
    send_text(&mut ws, &twap(3)).await;

    let gap: Value = serde_json::from_str(&h.record_of(RecordKind::Gap).await.d).unwrap();
    assert_eq!(gap["source"], "polybolt");
    assert_eq!(gap["channel"], "price.crypto.twap");
    assert_eq!(gap["expected"], 2);
    assert_eq!(gap["got"], 3);
    // Both frames still produce their events.
    assert!(matches!(h.next_event().await, FeedEvent::FeedReset { .. }));
    assert!(matches!(h.next_event().await, FeedEvent::Oracle { .. }));
    assert!(matches!(h.next_event().await, FeedEvent::Oracle { .. }));

    h.stop(runner).await;
}

#[tokio::test]
async fn rtds_subscribes_per_asset_and_pings() {
    let (ctx, h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_rtds_legacy_with(
        ctx,
        srv.url.clone(),
        vec![Asset::Btc, Asset::Sol],
        fast_backoff(),
        RtdsTiming {
            ping: ms(50),
            ..RtdsTiming::default()
        },
    ));

    let mut ws = srv.accept().await;
    let sub = next_json(&mut ws).await;
    assert_eq!(sub["action"], "subscribe");
    let subs = sub["subscriptions"].as_array().unwrap();
    assert_eq!(subs.len(), 4);
    for symbol in ["btc/usd", "sol/usd"] {
        let filters = json!({ "symbol": symbol }).to_string();
        for topic in ["crypto_prices_twap_sixty", "crypto_prices_chainlink"] {
            assert!(
                subs.contains(&json!({"topic": topic, "type": "*", "filters": filters})),
                "missing {topic} {symbol} in {sub}"
            );
        }
    }
    assert_eq!(next_text(&mut ws).await, "ping");

    h.stop(runner).await;
}

#[tokio::test]
async fn rtds_reconnects_when_stale() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_rtds_legacy_with(
        ctx,
        srv.url.clone(),
        vec![Asset::Btc],
        fast_backoff(),
        RtdsTiming {
            stale_after: ms(150),
            ..RtdsTiming::default()
        },
    ));

    let mut ws = srv.accept().await;
    next_json(&mut ws).await;
    let _ws2 = srv.accept().await;
    assert!(h.record_of(RecordKind::Error).await.d.contains("no frames"));
    drop(ws);

    h.stop(runner).await;
}

#[tokio::test]
async fn coinbase_subscribes_on_connect_and_forwards_quotes() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_coinbase_with(
        ctx,
        srv.url.clone(),
        vec![Asset::Btc, Asset::Eth],
        fast_backoff(),
        CoinbaseTiming::default(),
    ));

    let mut ws = srv.accept().await;
    assert_eq!(
        next_json(&mut ws).await,
        json!({"type":"subscribe","product_ids":["BTC-USD","ETH-USD","USDT-USD"],"channels":["ticker","heartbeat"]})
    );
    send_text(
        &mut ws,
        r#"{"type":"ticker","product_id":"BTC-USD","price":"80000.10","best_bid":"80000.00","best_ask":"80000.10","time":"2026-10-26T19:21:38.936886Z"}"#,
    )
    .await;
    assert!(matches!(h.next_event().await, FeedEvent::FeedReset { .. }));
    assert!(matches!(
        h.next_event().await,
        FeedEvent::CexQuote {
            asset: Asset::Btc,
            ..
        }
    ));

    h.stop(runner).await;
}

#[tokio::test]
async fn binance_uses_combined_stream_and_reconnects_at_max_lifetime() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let runner = tokio::spawn(run_binance_with(
        ctx,
        srv.url.clone(),
        vec![Asset::Btc, Asset::Eth],
        fast_backoff(),
        BinanceTiming {
            max_lifetime: ms(150),
            ..BinanceTiming::default()
        },
    ));

    let (ws, path) = srv.accept_with_path().await;
    assert_eq!(
        path,
        "/stream?streams=btcusdt@bookTicker/btcusdt@trade/ethusdt@bookTicker/ethusdt@trade"
    );
    let open = h.record_of(RecordKind::Open).await;
    assert_eq!(open.d, format!("{}{path}", srv.url));
    let _ws2 = srv.accept().await;
    h.record_of(RecordKind::Close).await;
    drop(ws);

    h.stop(runner).await;
}

#[tokio::test]
async fn status_marks_stale_and_counts_reconnects() {
    let (ctx, mut h) = harness();
    let srv = server().await;
    let (_desired, runner) = spawn_clob(ctx, &srv.url, &[], ClobTiming::default());

    let mut ws = srv.accept().await;
    send_text(&mut ws, "PONG").await;
    h.record_of(RecordKind::Text).await;
    let live = h.status.snapshot(raki_core::now_ms());
    let clob = live
        .iter()
        .find(|f| f.source == Source::ClobMarket)
        .unwrap();
    assert!(clob.connected && !clob.stale && clob.msgs_total == 1);

    // Drop the socket and stop listening, so reconnects fail.
    drop(ws);
    drop(srv);
    h.record_of(RecordKind::Gap).await;

    let snap = h.status.snapshot(raki_core::now_ms());
    let clob = snap
        .iter()
        .find(|f| f.source == Source::ClobMarket)
        .unwrap();
    assert!(!clob.connected);
    assert!(clob.stale);
    assert!(clob.reconnects >= 1);
    assert_eq!(clob.msgs_total, 1);
    assert!(clob.last_error.is_some());
    let other = snap.iter().find(|f| f.source == Source::Binance).unwrap();
    assert!(!other.connected && other.stale && other.reconnects == 0);

    h.stop(runner).await;
}

/// Runs the real feeds for 8 s. Needs network access:
/// `cargo test -p raki-feeds --test runners -- --ignored live_feeds_smoke`.
#[tokio::test]
#[ignore = "connects to the real feeds"]
async fn live_feeds_smoke() {
    ensure_crypto();
    let (rec_tx, mut rec_rx) = record_channel(200_000);
    let (ev_tx, mut ev_rx) = mpsc::channel(200_000);
    let enabled = SOURCES.map(|s| (s, true));
    let status = FeedStatusRegistry::new(&enabled, HashMap::new());
    let (reconnect_tx, _) = broadcast::channel::<()>(4);
    let shutdown = CancellationToken::new();
    let ctx = || FeedCtx {
        rec: rec_tx.clone(),
        events: ev_tx.clone(),
        status: status.clone(),
        reconnect: reconnect_tx.subscribe(),
        shutdown: shutdown.clone(),
    };
    let assets = Asset::ALL.to_vec();
    let (_desired, desired_rx) = watch::channel(BTreeSet::new());
    let runners = [
        tokio::spawn(run_clob(
            ctx(),
            "wss://ws-subscriptions-clob.polymarket.com/ws/market".into(),
            desired_rx,
        )),
        tokio::spawn(run_rtds_legacy(
            ctx(),
            "wss://ws-live-data.polymarket.com".into(),
            assets.clone(),
        )),
        tokio::spawn(run_binance(
            ctx(),
            "wss://stream.binance.com:9443".into(),
            assets.clone(),
        )),
        tokio::spawn(run_coinbase(
            ctx(),
            "wss://ws-feed.exchange.coinbase.com".into(),
            assets,
        )),
    ];
    drop((rec_tx, ev_tx));
    let drain = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });

    tokio::time::sleep(Duration::from_secs(8)).await;
    shutdown.cancel();
    for r in runners {
        timeout(WAIT, r).await.unwrap().unwrap();
    }

    let mut recs = Vec::new();
    while let Some(r) = rec_rx.recv().await {
        recs.push((r.s, r.k));
    }
    drain.await.unwrap();
    let n = |s, k| recs.iter().filter(|&&r| r == (s, k)).count();
    for s in SOURCES {
        let kinds = [
            RecordKind::Open,
            RecordKind::Send,
            RecordKind::Text,
            RecordKind::Close,
            RecordKind::Error,
        ];
        eprintln!("{s:?}: {:?}", kinds.map(|k| (k, n(s, k))));
    }
    // One connection held for the whole run: an unsubscribed CLOB socket must not be closed
    // by the server (it closes with 1008 on a PING before any subscribe).
    assert_eq!(n(Source::ClobMarket, RecordKind::Open), 1);
    assert_eq!(n(Source::ClobMarket, RecordKind::Error), 0);
    for s in [Source::RtdsLegacy, Source::Binance, Source::Coinbase] {
        assert!(n(s, RecordKind::Text) >= 1, "{s:?} recorded no text");
    }
}
