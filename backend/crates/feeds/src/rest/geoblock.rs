//! Polymarket geoblock check.

use raki_core::Source;
use serde::Deserialize;
use serde_json::Value;

use super::{PolyRest, RestError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoResult {
    pub blocked: bool,
    pub country: String,
    pub region: Option<String>,
}

#[derive(Deserialize)]
struct Wire {
    blocked: bool,
    country: String,
    region: Option<String>,
}

impl PolyRest {
    /// `GET {geoblock_url}`. The recorded body has the `ip` field removed: the raw stream must not
    /// hold our public address.
    pub async fn geoblock(&self) -> Result<GeoResult, RestError> {
        let (url, resp) = self.get(self.http.get(self.geoblock_url())).await?;
        let mut body: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
        if let Some(o) = body.as_object_mut() {
            o.remove("ip");
        }
        // A body that is not JSON is recorded as null rather than raw, since it could hold the ip.
        self.record_http(Source::Geoblock, &url, resp.status, body);
        if !(200..300).contains(&resp.status) {
            return Err(RestError::Status(resp.status));
        }
        let w: Wire = serde_json::from_str(&resp.body)?;
        Ok(GeoResult {
            blocked: w.blocked,
            country: w.country,
            region: w.region,
        })
    }
}
