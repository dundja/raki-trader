//! REST client tests: wiremock serving the real captures in `backend/fixtures`.

use std::path::Path;

use raki_core::{GammaWindow, Outcome, Source};
use raki_feeds::rest::{PolyRest, RestUrls, parse_gamma_event};
use raki_recorder::{Record, RecordKind, record_channel};
use serde_json::Value;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

/// Matches requests carrying a numeric `cb` query parameter.
struct HasCacheBuster;

impl Match for HasCacheBuster {
    fn matches(&self, req: &Request) -> bool {
        req.url
            .query_pairs()
            .any(|(k, v)| k == "cb" && v.parse::<i64>().is_ok_and(|ms| ms > 1_700_000_000_000))
    }
}

fn fixture(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn client(server: &MockServer, rec: Option<raki_recorder::RecordTx>) -> PolyRest {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let base = server.uri();
    PolyRest::new(
        RestUrls {
            gamma_base: base.clone(),
            data_base: base.clone(),
            geoblock_url: format!("{base}/api/geoblock"),
        },
        rec,
    )
}

fn json_ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn gamma(rel: &str) -> GammaWindow {
    parse_gamma_event(&fixture(rel)).unwrap()
}

#[test]
fn gamma_resolved_fixture() {
    let raw = fixture("gamma/btc_1793029800.json");
    let w = parse_gamma_event(&raw).unwrap();
    assert_eq!(w.slug, "btc-updown-5m-1793029800");
    assert_eq!(w.start, 1793029800);
    assert_eq!(w.end, 1793030100);
    assert_eq!(w.price_to_beat, Some(80584.595188212));
    assert_eq!(w.final_price, Some(80399.2702888047));
    assert!(w.closed);
    assert_eq!(w.twap_lookback_s, Some(60));
    assert_eq!(w.fee_rate, Some(0.07));
    assert_eq!(w.tick_size, Some(0.001));
    assert_eq!(
        w.condition_id,
        "0xa7d83c9fafffeb7efb196ecb527904892095d198649e841f2be1511398204d3e"
    );

    let v: Value = serde_json::from_str(&raw).unwrap();
    let ids: Vec<String> =
        serde_json::from_str(v["markets"][0]["clobTokenIds"].as_str().unwrap()).unwrap();
    assert_eq!(w.up_token, ids[0]);
    assert_eq!(w.down_token, ids[1]);
}

#[test]
fn gamma_live_fixture_has_no_metadata() {
    let w = gamma("gamma/btc_1793031000.json");
    assert_eq!(w.start, 1793031000);
    assert_eq!(w.price_to_beat, None);
    assert_eq!(w.final_price, None);
    assert!(!w.closed);
}

#[test]
fn gamma_eth_sol_fixtures_parse() {
    let eth = gamma("gamma/eth_sample.json");
    assert!(eth.slug.starts_with("eth-"), "{}", eth.slug);
    let sol = gamma("gamma/sol_sample.json");
    assert!(sol.slug.starts_with("sol-"), "{}", sol.slug);
    assert_eq!(eth.tick_size, Some(0.01));
}

#[test]
fn gamma_start_falls_back_to_event_start_time() {
    let mut v: Value = serde_json::from_str(&fixture("gamma/btc_1793029800.json")).unwrap();
    v.as_object_mut().unwrap().remove("startTime");
    let w = parse_gamma_event(&v.to_string()).unwrap();
    assert_eq!(w.start, 1793029800, "must not fall back to startDate");
}

#[test]
fn gamma_rejects_outcome_order_other_than_up_down() {
    let mut v: Value = serde_json::from_str(&fixture("gamma/btc_1793029800.json")).unwrap();
    v["markets"][0]["outcomes"] = Value::String(r#"["Down", "Up"]"#.into());
    let err = parse_gamma_event(&v.to_string()).unwrap_err();
    assert!(err.to_string().contains("Up"), "{err}");
}

#[tokio::test]
async fn gamma_404_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/events/slug/bad"))
        .respond_with(
            ResponseTemplate::new(404).set_body_raw(fixture("gamma/bad.json"), "application/json"),
        )
        .mount(&server)
        .await;
    assert_eq!(
        client(&server, None).gamma_window("bad").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn gamma_server_error_is_err() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    assert!(client(&server, None).gamma_window("x").await.is_err());
}

#[tokio::test]
async fn gamma_adds_cache_buster() {
    let server = MockServer::start().await;
    // Without the `cb` param wiremock would answer 404, i.e. `Ok(None)`.
    Mock::given(method("GET"))
        .and(path("/events/slug/btc-updown-5m-1793029800"))
        .and(HasCacheBuster)
        .respond_with(json_ok(fixture("gamma/btc_1793029800.json")))
        .expect(1)
        .mount(&server)
        .await;
    let w = client(&server, None)
        .gamma_window("btc-updown-5m-1793029800")
        .await
        .unwrap()
        .expect("cb param missing");
    assert_eq!(w.start, 1793029800);
}

#[tokio::test]
async fn gamma_response_is_recorded_as_http() {
    let server = MockServer::start().await;
    let body = fixture("gamma/btc_1793031000.json");
    Mock::given(method("GET"))
        .respond_with(json_ok(body.clone()))
        .mount(&server)
        .await;
    let (tx, mut rx) = record_channel(8);
    client(&server, Some(tx))
        .gamma_window("btc-updown-5m-1793031000")
        .await
        .unwrap();
    let rec: Record = rx.try_recv().unwrap();
    assert_eq!((rec.s, rec.k), (Source::Gamma, RecordKind::Http));
    let d: Value = serde_json::from_str(&rec.d).unwrap();
    assert_eq!(d["status"], 200);
    assert_eq!(d["body"], body);
    assert!(
        d["url"]
            .as_str()
            .unwrap()
            .contains("/events/slug/btc-updown-5m-1793031000?cb=")
    );
}

#[tokio::test]
async fn resolution_resolved_and_pending() {
    let server = MockServer::start().await;
    let resolved = fixture("data_api/resolution_resolved.json");
    let v: Value = serde_json::from_str(&resolved).unwrap();
    let cond = v["data"][0]["condition_id"].as_str().unwrap().to_string();
    let expected =
        chrono::DateTime::parse_from_rfc3339(v["data"][0]["resolved_at"].as_str().unwrap())
            .unwrap()
            .timestamp();
    assert!((1793040000..1793042000).contains(&expected), "{expected}");

    Mock::given(method("GET"))
        .and(path("/v2/resolutions"))
        .and(query_param("condition", cond.as_str()))
        .respond_with(json_ok(resolved))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/resolutions"))
        .and(query_param("condition", "0xpending"))
        .respond_with(json_ok(fixture("data_api/resolution_pending.json")))
        .mount(&server)
        .await;

    let c = client(&server, None);
    assert_eq!(
        c.resolution(&cond).await.unwrap(),
        Some((Outcome::Down, expected))
    );
    assert_eq!(c.resolution("0xpending").await.unwrap(), None);
}

#[tokio::test]
async fn resolution_up_when_up_payout_positive() {
    let server = MockServer::start().await;
    let body = fixture("data_api/resolution_resolved.json").replace("[0,1000000]", "[1000000,0]");
    Mock::given(method("GET"))
        .respond_with(json_ok(body))
        .mount(&server)
        .await;
    let got = client(&server, None).resolution("0xabc").await.unwrap();
    assert_eq!(got.map(|g| g.0), Some(Outcome::Up));
}

#[tokio::test]
async fn resolution_split_payout_is_err() {
    let server = MockServer::start().await;
    let body =
        fixture("data_api/resolution_resolved.json").replace("[0,1000000]", "[500000,500000]");
    Mock::given(method("GET"))
        .respond_with(json_ok(body))
        .mount(&server)
        .await;
    assert!(client(&server, None).resolution("0xabc").await.is_err());
}

#[tokio::test]
async fn geoblock_parses_and_record_strips_ip() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/geoblock"))
        .respond_with(json_ok(fixture("misc/geoblock.json")))
        .mount(&server)
        .await;
    let (tx, mut rx) = record_channel(8);
    let g = client(&server, Some(tx)).geoblock().await.unwrap();
    assert!(!g.blocked);
    assert_eq!(g.country, "RS");
    assert_eq!(g.region.as_deref(), Some("SE"));

    let rec = rx.try_recv().unwrap();
    assert_eq!((rec.s, rec.k), (Source::Geoblock, RecordKind::Http));
    assert!(!rec.d.contains("203.0.113.7"), "{}", rec.d);
    let d: Value = serde_json::from_str(&rec.d).unwrap();
    assert_eq!(d["status"], 200);
    assert_eq!(d["body"]["country"], "RS");
    assert!(d["body"].get("ip").is_none());
}
