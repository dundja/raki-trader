//! Parser tests against the real captures in `backend/fixtures`.

use std::path::PathBuf;

use raki_core::{Asset, FeedEvent, Level, OracleKind, Side, Source, Venue};
use raki_feeds::parse::clob::parse_clob;
use raki_feeds::parse::polybolt::{PolyboltFrame, PolyboltSeq, parse_polybolt, polybolt_seq};
use raki_feeds::parse::rtds::parse_rtds;
use raki_feeds::parse::{binance::parse_binance, coinbase::parse_coinbase};
use rust_decimal::Decimal;

fn fixture(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// `(recv_ms, raw)` for every line of a `{"recv_ms","raw"}` JSONL fixture.
fn lines(path: &str) -> Vec<(i64, String)> {
    fixture(path)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (
                v["recv_ms"].as_i64().unwrap(),
                v["raw"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// `(name, raw)` for every line of the PolyBolt AsyncAPI examples.
fn named_lines(path: &str) -> Vec<(String, String)> {
    fixture(path)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (
                v["name"].as_str().unwrap().to_owned(),
                v["raw"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn parse_all(
    path: &str,
    f: impl Fn(&str, i64) -> Result<Vec<FeedEvent>, raki_feeds::parse::ParseError>,
) -> Vec<FeedEvent> {
    lines(path)
        .into_iter()
        .flat_map(|(ms, raw)| f(&raw, ms).unwrap_or_else(|e| panic!("{path}: {e}: {raw:.200}")))
        .collect()
}

#[test]
fn clob_fixture_parses_without_errors() {
    let mut events = parse_all("clob/live_window.jsonl", parse_clob);
    let count =
        |evs: &[FeedEvent], pick: fn(&FeedEvent) -> bool| evs.iter().filter(|e| pick(e)).count();
    assert!(count(&events, |e| matches!(e, FeedEvent::Book { .. })) >= 2);
    assert!(count(&events, |e| matches!(e, FeedEvent::PriceChange { .. })) > 500);
    assert!(count(&events, |e| matches!(e, FeedEvent::LastTrade { .. })) >= 1);
    assert!(count(&events, |e| matches!(e, FeedEvent::BestBidAsk { .. })) >= 1);
    events = parse_all("clob/after_window_end.jsonl", parse_clob);
    assert!(!events.is_empty());
}

#[test]
fn clob_snapshot_array_yields_two_books_with_best_last() {
    let (ms, raw) = lines("clob/live_window.jsonl").remove(0);
    assert!(raw.starts_with('['));
    let events = parse_clob(&raw, ms).unwrap();
    assert_eq!(events.len(), 2);
    for e in &events {
        let FeedEvent::Book {
            bids,
            asks,
            ts_ms,
            market,
            asset_id,
        } = e
        else {
            panic!("expected Book, got {e:?}");
        };
        assert_eq!(*ts_ms, 1793031216412);
        assert!(market.starts_with("0x6fc2dbad"));
        assert!(!asset_id.is_empty());
        let max_bid = bids.iter().map(|l| l.price).max().unwrap();
        let min_ask = asks.iter().map(|l| l.price).min().unwrap();
        assert_eq!(
            bids.last().unwrap().price,
            max_bid,
            "bids ascending: best last"
        );
        assert_eq!(
            asks.last().unwrap().price,
            min_ask,
            "asks descending: best last"
        );
    }
    let FeedEvent::Book { bids, asks, .. } = &events[0] else {
        unreachable!()
    };
    assert_eq!(
        bids.first(),
        Some(&Level {
            price: dec("0.01"),
            size: dec("9346")
        })
    );
    assert_eq!(
        asks.first(),
        Some(&Level {
            price: dec("0.99"),
            size: dec("9152.55")
        })
    );
}

#[test]
fn clob_price_change_best_bid_ask_last_trade_fields() {
    let pc = r#"{"market":"0xm", "price_changes":[{"asset_id":"a1", "price":"0.02", "size":"0", "side":"BUY", "hash":"h", "best_bid":"0.05", "best_ask":"0.06"}, {"asset_id":"a2", "price":"0.98", "size":"4126.32", "side":"SELL", "hash":"h2", "best_bid":"0", "best_ask":"1"}], "timestamp":"1793031216412", "event_type":"price_change"}"#;
    let evs = parse_clob(pc, 1).unwrap();
    assert_eq!(evs.len(), 1);
    let FeedEvent::PriceChange {
        market,
        ts_ms,
        changes,
    } = &evs[0]
    else {
        panic!()
    };
    assert_eq!((market.as_str(), *ts_ms), ("0xm", 1793031216412));
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].size, Decimal::ZERO, "size 0 is kept");
    assert_eq!(changes[0].side, Side::Buy);
    assert_eq!(changes[0].best_bid, Some(dec("0.05")));
    assert_eq!(changes[1].side, Side::Sell);

    let bba = r#"{"market":"0xm", "asset_id":"a1", "best_bid":"", "best_ask":"0.01", "spread":"0.01", "timestamp":"5", "event_type":"best_bid_ask"}"#;
    assert_eq!(
        parse_clob(bba, 1).unwrap(),
        vec![FeedEvent::BestBidAsk {
            asset_id: "a1".into(),
            ts_ms: 5,
            best_bid: None,
            best_ask: Some(dec("0.01"))
        }]
    );
    let bba_absent = r#"{"market":"0xm", "asset_id":"a1", "best_ask":"0.01", "timestamp":"5", "event_type":"best_bid_ask"}"#;
    assert!(matches!(
        parse_clob(bba_absent, 1).unwrap()[..],
        [FeedEvent::BestBidAsk {
            best_bid: None,
            best_ask: Some(_),
            ..
        }]
    ));

    let lt = r#"{"market":"0xm", "asset_id":"a1", "price":"0.95", "size":"6.3263", "fee_rate_bps":"0", "side":"SELL", "timestamp":"7", "event_type":"last_trade_price", "transaction_hash":"0xt"}"#;
    assert_eq!(
        parse_clob(lt, 1).unwrap(),
        vec![FeedEvent::LastTrade {
            asset_id: "a1".into(),
            ts_ms: 7,
            price: dec("0.95"),
            size: dec("6.3263"),
            side: Side::Sell
        }]
    );
}

#[test]
fn clob_control_and_unknown_frames_yield_nothing() {
    assert_eq!(parse_clob("PONG", 1).unwrap(), vec![]);
    assert_eq!(
        parse_clob(r#"{"event_type":"new_market","market":"0x1"}"#, 1).unwrap(),
        vec![]
    );
    assert_eq!(
        parse_clob(r#"{"event_type":"something_new","x":1}"#, 1).unwrap(),
        vec![]
    );
    let (ms, raw) = lines("clob/after_window_end.jsonl")
        .into_iter()
        .find(|(_, r)| r.contains("\"new_market\""))
        .unwrap();
    assert_eq!(parse_clob(&raw, ms).unwrap(), vec![]);
}

#[test]
fn clob_market_resolved() {
    let (ms, raw) = lines("clob/market_resolved.jsonl").remove(0);
    assert_eq!(
        parse_clob(&raw, ms).unwrap(),
        vec![FeedEvent::MarketResolved {
            condition_id: "0x1e5fd68c1285adeba23cf896df30b56cf112e0d0c788a3eb7bcbdaeb0ffa2d82"
                .into(),
            winning_asset_id:
                "44338029085680197241344871695915829650518704519111863069984122817290013415816"
                    .into(),
            winning_outcome: "Up".into(),
            recv_ms: ms,
        }]
    );
    // The documented alternative spelling of the asset list is accepted too.
    let alt = raw.replace("assets_ids", "asset_ids");
    assert_eq!(parse_clob(&alt, ms).unwrap().len(), 1);
}

#[test]
fn clob_tick_size_change_parsed() {
    let events = parse_all("clob/after_window_end.jsonl", parse_clob);
    let ticks: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            FeedEvent::TickSize { new_tick, .. } => Some(*new_tick),
            _ => None,
        })
        .collect();
    assert!(ticks.len() >= 2, "got {}", ticks.len());
    assert!(ticks.iter().all(|t| *t == dec("0.001")));
}

#[test]
fn rtds_twap_at_boundary_matches_gamma() {
    let events = parse_all("rtds/boundary.jsonl", parse_rtds);
    let hit = events.iter().any(|e| {
        matches!(
            e,
            FeedEvent::Oracle { asset: Asset::Btc, kind: OracleKind::Twap60, source: Source::RtdsLegacy, ts_ms: 1793039400000, value }
                if *value == 80121.11048035804
        )
    });
    assert!(hit, "no BTC Twap60 point at the boundary");
    // Snapshots expand into one event per point.
    assert!(events.len() > 100);
}

#[test]
fn rtds_empty_frame_ok() {
    assert_eq!(parse_rtds("", 0).unwrap(), vec![]);
}

#[test]
fn rtds_chainlink_and_binance_relay_routing() {
    let cl = r#"{"connection_id":"x","topic":"crypto_prices_chainlink","type":"update","timestamp":1793031306211,"payload":{"symbol":"btc/usd","timestamp":1793031305000,"value":80107.14667171605,"full_accuracy_value":"80107146671716050000000"}}"#;
    assert_eq!(
        parse_rtds(cl, 1).unwrap(),
        vec![FeedEvent::Oracle {
            asset: Asset::Btc,
            kind: OracleKind::Chainlink,
            source: Source::RtdsLegacy,
            ts_ms: 1793031305000,
            value: 80107.14667171605
        }]
    );
    // The Chainlink snapshot arrives under the topic `crypto_prices`.
    let snap = r#"{"topic":"crypto_prices","type":"subscribe","timestamp":1793031329784,"payload":{"symbol":"sol/usd","data":[{"timestamp":1793031270000,"value":101.5},{"timestamp":1793031271000,"value":101.6}]}}"#;
    let evs = parse_rtds(snap, 1).unwrap();
    assert_eq!(evs.len(), 2);
    assert!(evs.iter().all(|e| matches!(
        e,
        FeedEvent::Oracle {
            asset: Asset::Sol,
            kind: OracleKind::Chainlink,
            ..
        }
    )));
    // The Binance relay is ignored: we take Binance directly.
    let relay = r#"{"connection_id":"x","topic":"crypto_prices","type":"update","timestamp":1793031363121,"payload":{"symbol":"btcusdt","timestamp":1793031363000,"value":80240.1,"full_accuracy_value":"80240.10000000"}}"#;
    assert_eq!(parse_rtds(relay, 1).unwrap(), vec![]);
    // Symbols we do not track are ignored.
    let xrp = cl.replace("btc/usd", "xrp/usd");
    assert_eq!(parse_rtds(&xrp, 1).unwrap(), vec![]);
}

#[test]
fn polybolt_examples() {
    let mut seen = 0;
    for (name, raw) in named_lines("polybolt/asyncapi_examples.jsonl") {
        let frame = parse_polybolt(&raw, 1).unwrap_or_else(|e| panic!("{name}: {e}"));
        seen += 1;
        match name.as_str() {
            "priceCryptoTwap:live" => assert_eq!(
                frame,
                PolyboltFrame::Events(vec![FeedEvent::Oracle {
                    asset: Asset::Btc,
                    kind: OracleKind::Twap60,
                    source: Source::Polybolt,
                    ts_ms: 1788973002000,
                    value: 64120.7,
                }])
            ),
            "priceCrypto:live" => assert_eq!(
                frame,
                PolyboltFrame::Events(vec![FeedEvent::Oracle {
                    asset: Asset::Btc,
                    kind: OracleKind::Chainlink,
                    source: Source::Polybolt,
                    ts_ms: 1788973001000,
                    value: 64126.0,
                }])
            ),
            "priceCrypto:snapshot" => {
                let PolyboltFrame::Events(evs) = frame else {
                    panic!("{frame:?}")
                };
                assert_eq!(evs.len(), 2);
            }
            n if n.starts_with("priceCryptoTwap:snapshot") => {
                let PolyboltFrame::Events(evs) = frame else {
                    panic!("{frame:?}")
                };
                assert_eq!(evs.len(), 2);
                assert!(evs.iter().all(|e| matches!(
                    e,
                    FeedEvent::Oracle {
                        kind: OracleKind::Twap60,
                        source: Source::Polybolt,
                        ..
                    }
                )));
            }
            "authed:authed" => assert_eq!(frame, PolyboltFrame::Authed),
            "subscribed:subscribed" => assert_eq!(
                frame,
                PolyboltFrame::Subscribed {
                    channel: "price.crypto".into()
                }
            ),
            "pong:pong" => assert_eq!(frame, PolyboltFrame::Pong),
            "error:auth_required" => assert_eq!(
                frame,
                PolyboltFrame::Error {
                    code: "auth_required".into()
                }
            ),
            "error:bad_filter" => assert_eq!(
                frame,
                PolyboltFrame::Error {
                    code: "bad_filter".into()
                }
            ),
            "unsubscribed:unsubscribed" => assert_eq!(frame, PolyboltFrame::Other),
            _ => {}
        }
    }
    assert_eq!(seen, 11);
}

#[test]
fn polybolt_seq_gap_detected() {
    let mut s = PolyboltSeq::default();
    assert_eq!(s.check("price.crypto", 1), None);
    assert_eq!(s.check("price.crypto", 2), None);
    assert_eq!(s.check("price.crypto", 5), Some((3, 5)));
    assert_eq!(s.check("price.crypto", 6), None, "gap is reported once");
    // Channels are tracked independently.
    assert_eq!(s.check("price.crypto.twap", 40), None);
    s.reset();
    assert_eq!(s.check("price.crypto", 100), None);
}

#[test]
fn polybolt_seq_extracted_from_data_frames_only() {
    let rows = named_lines("polybolt/asyncapi_examples.jsonl");
    let raw = |name: &str| rows.iter().find(|(n, _)| n == name).unwrap().1.clone();
    assert_eq!(
        polybolt_seq(&raw("priceCryptoTwap:live")),
        Some(("price.crypto.twap".into(), 3))
    );
    assert_eq!(
        polybolt_seq(&raw("priceCrypto:snapshot")),
        Some(("price.crypto".into(), 1))
    );
    assert_eq!(polybolt_seq(&raw("authed:authed")), None);
    assert_eq!(polybolt_seq("{not json"), None);
}

#[test]
fn binance_book_ticker() {
    let events = parse_all("cex/binance.jsonl", parse_binance);
    let mut assets = std::collections::BTreeSet::new();
    for e in &events {
        let FeedEvent::CexQuote {
            venue,
            asset,
            bid,
            ask,
            ..
        } = e
        else {
            panic!("unexpected {e:?}");
        };
        assert_eq!(*venue, Venue::Binance);
        assert!(bid < ask, "{asset:?}: {bid} !< {ask}");
        assets.insert(*asset);
    }
    assert_eq!(
        assets.into_iter().collect::<Vec<_>>(),
        vec![Asset::Btc, Asset::Eth, Asset::Sol]
    );
    // Trades produce nothing (recorded raw only).
    let trade = r#"{"stream":"ethusdt@trade","data":{"e":"trade","E":1,"s":"ETHUSDT","t":4,"p":"2319.83000000","q":"0.0035","T":1,"m":true,"M":true}}"#;
    assert_eq!(parse_binance(trade, 1).unwrap(), vec![]);
}

#[test]
fn coinbase_ticker_and_usdt() {
    let events = parse_all("cex/coinbase.jsonl", parse_coinbase);
    let usdt: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            FeedEvent::UsdtUsd { price, .. } => Some(*price),
            _ => None,
        })
        .collect();
    assert!(!usdt.is_empty());
    assert!(usdt.iter().all(|p| *p > dec("0.98") && *p < dec("1.02")));
    assert!(events.iter().any(|e| matches!(
        e,
        FeedEvent::CexQuote { venue: Venue::Coinbase, asset: Asset::Btc, bid, ask, .. } if bid <= ask
    )));
    // subscriptions ack and heartbeat are ignored.
    let (ms, ack) = lines("cex/coinbase.jsonl")
        .into_iter()
        .find(|(_, r)| r.contains("\"subscriptions\""))
        .unwrap();
    assert_eq!(parse_coinbase(&ack, ms).unwrap(), vec![]);
    let hb = r#"{"type":"heartbeat","sequence":1,"last_trade_id":2,"product_id":"BTC-USD","time":"2026-10-26T19:21:38.936886Z"}"#;
    assert_eq!(parse_coinbase(hb, 1).unwrap(), vec![]);
}

#[test]
fn garbage_is_error_not_panic() {
    assert!(parse_clob("{not json", 0).is_err());
    assert!(parse_clob("[1,2", 0).is_err());
    assert!(parse_rtds("{not json", 0).is_err());
    assert!(parse_polybolt("{not json", 0).is_err());
    assert!(parse_binance("{not json", 0).is_err());
    assert!(parse_coinbase("{not json", 0).is_err());
}
