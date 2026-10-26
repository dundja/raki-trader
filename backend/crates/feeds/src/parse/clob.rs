//! Polymarket CLOB market-channel frames.

use std::borrow::Cow;

use raki_core::{FeedEvent, Level, PriceChangeEntry, Side};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::{ParseError, ms_string, opt_decimal};

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum WireSide {
    Buy,
    Sell,
}

impl From<WireSide> for Side {
    fn from(s: WireSide) -> Self {
        match s {
            WireSide::Buy => Side::Buy,
            WireSide::Sell => Side::Sell,
        }
    }
}

#[derive(Deserialize)]
struct Tag<'a> {
    #[serde(borrow)]
    event_type: Cow<'a, str>,
}

#[derive(Deserialize)]
struct WireLevel {
    #[serde(with = "rust_decimal::serde::str")]
    price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    size: Decimal,
}

impl From<WireLevel> for Level {
    fn from(l: WireLevel) -> Self {
        Level {
            price: l.price,
            size: l.size,
        }
    }
}

#[derive(Deserialize)]
struct Book<'a> {
    #[serde(borrow)]
    market: Cow<'a, str>,
    #[serde(borrow)]
    asset_id: Cow<'a, str>,
    #[serde(deserialize_with = "ms_string")]
    timestamp: i64,
    bids: Vec<WireLevel>,
    asks: Vec<WireLevel>,
}

#[derive(Deserialize)]
struct PriceChange<'a> {
    #[serde(borrow)]
    market: Cow<'a, str>,
    #[serde(deserialize_with = "ms_string")]
    timestamp: i64,
    price_changes: Vec<WireChange<'a>>,
}

#[derive(Deserialize)]
struct WireChange<'a> {
    #[serde(borrow)]
    asset_id: Cow<'a, str>,
    #[serde(with = "rust_decimal::serde::str")]
    price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    size: Decimal,
    side: WireSide,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_bid: Option<Decimal>,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_ask: Option<Decimal>,
}

#[derive(Deserialize)]
struct BestBidAsk<'a> {
    #[serde(borrow)]
    asset_id: Cow<'a, str>,
    #[serde(deserialize_with = "ms_string")]
    timestamp: i64,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_bid: Option<Decimal>,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_ask: Option<Decimal>,
}

#[derive(Deserialize)]
struct LastTrade<'a> {
    #[serde(borrow)]
    asset_id: Cow<'a, str>,
    #[serde(deserialize_with = "ms_string")]
    timestamp: i64,
    #[serde(with = "rust_decimal::serde::str")]
    price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    size: Decimal,
    side: WireSide,
}

#[derive(Deserialize)]
struct TickSizeChange<'a> {
    #[serde(borrow)]
    asset_id: Cow<'a, str>,
    #[serde(deserialize_with = "ms_string")]
    timestamp: i64,
    #[serde(with = "rust_decimal::serde::str")]
    new_tick_size: Decimal,
}

/// The ids of a resolved market are in `assets_ids` (or `asset_ids`); we only need the winner.
#[derive(Deserialize)]
struct MarketResolved<'a> {
    #[serde(borrow)]
    market: Cow<'a, str>,
    #[serde(borrow)]
    winning_asset_id: Cow<'a, str>,
    #[serde(borrow)]
    winning_outcome: Cow<'a, str>,
}

/// Parses one CLOB WebSocket text frame: `PONG`, a single event object, or an array of them
/// (the initial subscribe snapshot is an array of `book`s).
pub fn parse_clob(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError> {
    let text = text.trim();
    if text == "PONG" {
        return Ok(vec![]);
    }
    if text.starts_with('[') {
        let objects: Vec<&RawValue> = serde_json::from_str(text)?;
        let mut out = Vec::with_capacity(objects.len());
        for obj in objects {
            out.extend(parse_object(obj.get(), recv_ms)?);
        }
        return Ok(out);
    }
    parse_object(text, recv_ms)
}

fn parse_object(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError> {
    let tag: Tag = serde_json::from_str(text)?;
    Ok(match tag.event_type.as_ref() {
        "book" => {
            let b: Book = serde_json::from_str(text)?;
            vec![FeedEvent::Book {
                asset_id: b.asset_id.into_owned(),
                market: b.market.into_owned(),
                ts_ms: b.timestamp,
                bids: b.bids.into_iter().map(Level::from).collect(),
                asks: b.asks.into_iter().map(Level::from).collect(),
            }]
        }
        "price_change" => {
            let p: PriceChange = serde_json::from_str(text)?;
            vec![FeedEvent::PriceChange {
                market: p.market.into_owned(),
                ts_ms: p.timestamp,
                changes: p
                    .price_changes
                    .into_iter()
                    .map(|c| PriceChangeEntry {
                        asset_id: c.asset_id.into_owned(),
                        price: c.price,
                        size: c.size,
                        side: c.side.into(),
                        best_bid: c.best_bid,
                        best_ask: c.best_ask,
                    })
                    .collect(),
            }]
        }
        "best_bid_ask" => {
            let b: BestBidAsk = serde_json::from_str(text)?;
            vec![FeedEvent::BestBidAsk {
                asset_id: b.asset_id.into_owned(),
                ts_ms: b.timestamp,
                best_bid: b.best_bid,
                best_ask: b.best_ask,
            }]
        }
        "last_trade_price" => {
            let t: LastTrade = serde_json::from_str(text)?;
            vec![FeedEvent::LastTrade {
                asset_id: t.asset_id.into_owned(),
                ts_ms: t.timestamp,
                price: t.price,
                size: t.size,
                side: t.side.into(),
            }]
        }
        "tick_size_change" => {
            let t: TickSizeChange = serde_json::from_str(text)?;
            vec![FeedEvent::TickSize {
                asset_id: t.asset_id.into_owned(),
                ts_ms: t.timestamp,
                new_tick: t.new_tick_size,
            }]
        }
        "market_resolved" => {
            let m: MarketResolved = serde_json::from_str(text)?;
            vec![FeedEvent::MarketResolved {
                condition_id: m.market.into_owned(),
                winning_asset_id: m.winning_asset_id.into_owned(),
                winning_outcome: m.winning_outcome.into_owned(),
                recv_ms,
            }]
        }
        // `new_market` is broadcast for unrelated markets; anything else is not ours either.
        _ => vec![],
    })
}
