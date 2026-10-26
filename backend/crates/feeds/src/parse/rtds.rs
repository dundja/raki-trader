//! Legacy Polymarket RTDS (`wss://ws-live-data.polymarket.com`) frames.

use std::borrow::Cow;

use raki_core::{FeedEvent, OracleKind, Source};
use serde::Deserialize;

use super::{OraclePayload, ParseError};

#[derive(Deserialize)]
struct Frame<'a> {
    #[serde(borrow)]
    topic: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    payload: Option<OraclePayload<'a>>,
}

/// Parses one legacy RTDS text frame.
///
/// - `crypto_prices_twap_sixty` gives `Twap60`.
/// - `crypto_prices_chainlink` gives `Chainlink`. Its snapshot arrives under topic `crypto_prices`
///   with `type == "subscribe"` and a `base/quote` symbol, which is also `Chainlink`.
/// - Other `crypto_prices` frames are the Binance relay (`*usdt` symbols); we take Binance directly.
pub fn parse_rtds(text: &str, _recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let frame: Frame = serde_json::from_str(text)?;
    let kind = match (frame.topic.as_deref(), frame.kind.as_deref()) {
        (Some("crypto_prices_twap_sixty"), _) => OracleKind::Twap60,
        (Some("crypto_prices_chainlink"), _) => OracleKind::Chainlink,
        (Some("crypto_prices"), Some("subscribe")) => {
            let is_pair = frame
                .payload
                .as_ref()
                .and_then(|p| p.symbol.as_deref())
                .is_some_and(|s| s.contains('/'));
            if !is_pair {
                return Ok(vec![]);
            }
            OracleKind::Chainlink
        }
        _ => return Ok(vec![]),
    };
    match frame.payload {
        Some(payload) => payload.into_events(kind, Source::RtdsLegacy),
        None => Err(ParseError::Unexpected("price frame without payload".into())),
    }
}
