//! Binance combined-stream frames (`{"stream": "...", "data": {...}}`).

use std::borrow::Cow;

use raki_core::{Asset, FeedEvent, Venue};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::ParseError;

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    stream: Option<Cow<'a, str>>,
    #[serde(borrow)]
    data: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct BookTicker<'a> {
    #[serde(borrow, rename = "s")]
    symbol: Cow<'a, str>,
    #[serde(rename = "b", with = "rust_decimal::serde::str")]
    bid: Decimal,
    #[serde(rename = "a", with = "rust_decimal::serde::str")]
    ask: Decimal,
}

/// `*@bookTicker` gives a `CexQuote`. `*@trade` is recorded raw only and gives nothing.
pub fn parse_binance(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError> {
    let env: Envelope = serde_json::from_str(text)?;
    let (Some(stream), Some(data)) = (env.stream, env.data) else {
        return Ok(vec![]);
    };
    if !stream.ends_with("@bookTicker") {
        return Ok(vec![]);
    }
    let t: BookTicker = serde_json::from_str(data.get())?;
    let Some(asset) = Asset::from_any(&t.symbol) else {
        return Ok(vec![]);
    };
    Ok(vec![FeedEvent::CexQuote {
        venue: Venue::Binance,
        asset,
        bid: t.bid,
        ask: t.ask,
        recv_ms,
    }])
}
