//! Pure frame parsers.
//!
//! Every parser takes the exact text of one WebSocket frame plus the local receive time and returns
//! the typed events it carries. Control and ack frames yield no events. A parse failure is an
//! [`ParseError`], never a panic: callers write the raw frame to the recorder *before* parsing, so
//! an error never loses data.

use std::borrow::Cow;
use std::str::FromStr;

use raki_core::{Asset, FeedEvent, OracleKind, Source};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

pub mod binance;
pub mod clob;
pub mod coinbase;
pub mod polybolt;
pub mod rtds;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("json: {0}")]
    Json(String),
    #[error("unexpected: {0}")]
    Unexpected(String),
}

impl From<serde_json::Error> for ParseError {
    fn from(e: serde_json::Error) -> Self {
        ParseError::Json(e.to_string())
    }
}

/// Deserializes `Option<Decimal>` from a wire string. Absent, `null` and `""` all give `None`.
/// The value goes string to `Decimal` directly, never through `f64`.
pub(crate) fn opt_decimal<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Decimal>, D::Error> {
    let s = Option::<Cow<'de, str>>::deserialize(d)?;
    match s.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => Decimal::from_str(s)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

/// Deserializes an epoch-ms timestamp sent as a JSON string, e.g. `"1793031216412"`.
pub(crate) fn ms_string<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let s = Cow::<'de, str>::deserialize(d)?;
    s.parse().map_err(serde::de::Error::custom)
}

/// One oracle price point of a snapshot `data[]` array.
#[derive(Deserialize)]
pub(crate) struct OraclePoint {
    timestamp: i64,
    value: f64,
}

/// The `payload` shared by the legacy RTDS and PolyBolt price channels. A snapshot carries
/// `data[]`; a live update carries `timestamp` and `value` directly.
#[derive(Deserialize)]
pub(crate) struct OraclePayload<'a> {
    #[serde(borrow)]
    symbol: Option<Cow<'a, str>>,
    timestamp: Option<i64>,
    value: Option<f64>,
    data: Option<Vec<OraclePoint>>,
}

impl OraclePayload<'_> {
    /// One event per point. Symbols we do not track (xrp, doge, ...) give no events.
    pub(crate) fn into_events(
        self,
        kind: OracleKind,
        source: Source,
    ) -> Result<Vec<FeedEvent>, ParseError> {
        let symbol = self
            .symbol
            .ok_or_else(|| ParseError::Unexpected("price payload without symbol".into()))?;
        let Some(asset) = Asset::from_any(&symbol) else {
            return Ok(vec![]);
        };
        let event = |ts_ms, value| FeedEvent::Oracle {
            asset,
            kind,
            source,
            ts_ms,
            value,
        };
        if let Some(points) = self.data {
            return Ok(points
                .into_iter()
                .map(|p| event(p.timestamp, p.value))
                .collect());
        }
        match (self.timestamp, self.value) {
            (Some(ts), Some(value)) => Ok(vec![event(ts, value)]),
            _ => Err(ParseError::Unexpected(format!(
                "price payload for {symbol} has neither data nor timestamp+value"
            ))),
        }
    }
}
