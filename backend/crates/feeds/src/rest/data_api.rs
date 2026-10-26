//! Data API `/v2/resolutions`: the resolution authority.

use chrono::DateTime;
use raki_core::{Outcome, Source};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::{PolyRest, RestError};

#[derive(Deserialize)]
struct Resolutions {
    data: Vec<Resolution>,
}

#[derive(Deserialize)]
struct Resolution {
    /// `[up, down]` in micro-USDC.
    payouts: Vec<u64>,
    resolved_at: String,
}

fn parse_resolution(body: &str) -> Result<Option<(Outcome, i64)>, RestError> {
    let parsed: Resolutions = serde_json::from_str(body)?;
    let Some(r) = parsed.data.first() else {
        return Ok(None);
    };
    let winner = match r.payouts.as_slice() {
        [up, down] if *up > 0 && *down == 0 => Outcome::Up,
        [up, down] if *down > 0 && *up == 0 => Outcome::Down,
        other => {
            return Err(RestError::Unexpected(format!(
                "no single winner in payouts {other:?}"
            )));
        }
    };
    let at = DateTime::parse_from_rfc3339(&r.resolved_at)
        .map_err(|e| RestError::Unexpected(format!("resolved_at {:?}: {e}", r.resolved_at)))?;
    Ok(Some((winner, at.timestamp())))
}

impl PolyRest {
    /// `GET {data}/v2/resolutions?condition={id}`. An empty `data` array means not resolved yet.
    /// Returns the winner and the resolution time in unix seconds.
    pub async fn resolution(
        &self,
        condition_id: &str,
    ) -> Result<Option<(Outcome, i64)>, RestError> {
        let url = Url::parse_with_params(
            &format!("{}/v2/resolutions", self.data_base()),
            [("condition", condition_id)],
        )
        .map_err(|e| RestError::Unexpected(format!("data api url: {e}")))?;
        let (url, resp) = self.get(self.http.get(url)).await?;
        self.record_http(
            Source::DataApi,
            &url,
            resp.status,
            Value::String(resp.body.clone()),
        );
        if !(200..300).contains(&resp.status) {
            return Err(RestError::Status(resp.status));
        }
        parse_resolution(&resp.body)
    }
}
