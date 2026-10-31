//! Gamma event lookup and the pure event parser.

use chrono::DateTime;
use raki_core::{GammaWindow, Source, now_ms};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::{PolyRest, RestError};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    id: String,
    slug: String,
    /// RFC3339. Never fall back to `startDate`: that is the creation time of the event.
    start_time: Option<String>,
    end_date: String,
    #[serde(default)]
    closed: bool,
    event_metadata: Option<EventMetadata>,
    markets: Vec<Market>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventMetadata {
    price_to_beat: Option<f64>,
    final_price: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Market {
    condition_id: String,
    /// JSON-encoded string array.
    clob_token_ids: String,
    /// JSON-encoded string array.
    outcomes: String,
    event_start_time: Option<String>,
    crypto_market_config: Option<CryptoConfig>,
    fee_schedule: Option<FeeSchedule>,
    order_price_min_tick_size: Option<f64>,
    order_min_size: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CryptoConfig {
    twap_lookback_seconds: Option<i64>,
}

#[derive(Deserialize)]
struct FeeSchedule {
    rate: Option<f64>,
    exponent: Option<f64>,
}

fn unix_s(field: &str, s: &str) -> Result<i64, RestError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .map_err(|e| RestError::Unexpected(format!("{field} {s:?}: {e}")))
}

/// Parses a Gamma `/events/slug/{slug}` body into a [`GammaWindow`].
///
/// Outcomes are mapped by their string (`"Up"`/`"Down"`). If index 0 is not `"Up"` the event is
/// rejected so the caller can skip the window.
pub fn parse_gamma_event(body: &str) -> Result<GammaWindow, RestError> {
    let ev: Event = serde_json::from_str(body)?;
    let m = ev
        .markets
        .first()
        .ok_or_else(|| RestError::Unexpected("event has no markets".into()))?;

    let start = match (&ev.start_time, &m.event_start_time) {
        (Some(s), _) | (None, Some(s)) => unix_s("startTime", s)?,
        (None, None) => return Err(RestError::Unexpected("no startTime".into())),
    };
    let end = unix_s("endDate", &ev.end_date)?;

    let tokens: Vec<String> = serde_json::from_str(&m.clob_token_ids)?;
    let outcomes: Vec<String> = serde_json::from_str(&m.outcomes)?;
    if outcomes.first().map(String::as_str) != Some("Up") {
        return Err(RestError::Unexpected(format!(
            "outcome index 0 is not \"Up\": {outcomes:?}"
        )));
    }
    if tokens.len() != outcomes.len() {
        return Err(RestError::Unexpected(format!(
            "{} token ids for {} outcomes",
            tokens.len(),
            outcomes.len()
        )));
    }
    let token_of = |name: &str| {
        outcomes
            .iter()
            .position(|o| o == name)
            .map(|i| tokens[i].clone())
            .ok_or_else(|| RestError::Unexpected(format!("no {name:?} outcome: {outcomes:?}")))
    };

    Ok(GammaWindow {
        slug: ev.slug,
        event_id: ev.id,
        start,
        end,
        condition_id: m.condition_id.clone(),
        up_token: token_of("Up")?,
        down_token: token_of("Down")?,
        price_to_beat: ev.event_metadata.as_ref().and_then(|e| e.price_to_beat),
        final_price: ev.event_metadata.as_ref().and_then(|e| e.final_price),
        closed: ev.closed,
        twap_lookback_s: m
            .crypto_market_config
            .as_ref()
            .and_then(|c| c.twap_lookback_seconds),
        fee_rate: m.fee_schedule.as_ref().and_then(|f| f.rate),
        fee_exponent: m.fee_schedule.as_ref().and_then(|f| f.exponent),
        tick_size: m.order_price_min_tick_size,
        order_min_size: m.order_min_size,
    })
}

impl PolyRest {
    /// `GET {gamma}/events/slug/{slug}?cb={now_ms}`. The cache-buster is needed because Gamma
    /// responses are CDN-cached for up to 300 s. A 404 gives `Ok(None)`.
    pub async fn gamma_window(&self, slug: &str) -> Result<Option<GammaWindow>, RestError> {
        let url = Url::parse_with_params(
            &format!("{}/events/slug/{slug}", self.gamma_base()),
            [("cb", now_ms().to_string())],
        )
        .map_err(|e| RestError::Unexpected(format!("gamma url: {e}")))?;
        let (url, resp) = self.get(self.http.get(url)).await?;
        self.record_http(
            Source::Gamma,
            &url,
            resp.status,
            Value::String(resp.body.clone()),
        );
        match resp.status {
            404 => Ok(None),
            200..=299 => parse_gamma_event(&resp.body).map(Some),
            s => Err(RestError::Status(s)),
        }
    }
}
