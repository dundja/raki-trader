//! Coinbase Exchange feed frames.

use std::borrow::Cow;

use raki_core::{Asset, FeedEvent, Venue};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{ParseError, opt_decimal};

#[derive(Deserialize)]
struct Frame<'a> {
    #[serde(borrow, rename = "type")]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    product_id: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "opt_decimal")]
    price: Option<Decimal>,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_bid: Option<Decimal>,
    #[serde(default, deserialize_with = "opt_decimal")]
    best_ask: Option<Decimal>,
}

/// `ticker` for `USDT-USD` gives `UsdtUsd`; other tracked products give a `CexQuote` from the
/// ticker's best bid/ask. `heartbeat`, `subscriptions` and anything else give nothing.
pub fn parse_coinbase(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError> {
    let f: Frame = serde_json::from_str(text)?;
    if f.kind != "ticker" {
        return Ok(vec![]);
    }
    let product = f
        .product_id
        .ok_or_else(|| ParseError::Unexpected("ticker without product_id".into()))?;
    if product == "USDT-USD" {
        let price = f
            .price
            .ok_or_else(|| ParseError::Unexpected("USDT-USD ticker without price".into()))?;
        return Ok(vec![FeedEvent::UsdtUsd { price, recv_ms }]);
    }
    let Some(asset) = Asset::from_any(&product) else {
        return Ok(vec![]);
    };
    match (f.best_bid, f.best_ask) {
        (Some(bid), Some(ask)) => Ok(vec![FeedEvent::CexQuote {
            venue: Venue::Coinbase,
            asset,
            bid,
            ask,
            recv_ms,
        }]),
        _ => Err(ParseError::Unexpected(format!(
            "{product} ticker without best_bid/best_ask"
        ))),
    }
}
