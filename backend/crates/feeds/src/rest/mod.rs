//! REST clients: Gamma (window metadata), Data API (resolution authority) and the geoblock check.
//!
//! Every HTTP response body is recorded as a [`RecordKind::Http`] record before it is parsed, so a
//! parse failure never loses the capture. The `d` field is `{"url":..,"status":..,"body":..}`.

use std::time::Duration;

use raki_core::{Source, now_ns};
use raki_recorder::{Record, RecordKind, RecordTx};
use serde_json::json;

pub mod data_api;
pub mod gamma;
pub mod geoblock;

pub use gamma::parse_gamma_event;
pub use geoblock::GeoResult;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum RestError {
    #[error("http: {0}")]
    Http(String),
    #[error("unexpected status {0}")]
    Status(u16),
    #[error("json: {0}")]
    Json(String),
    #[error("unexpected: {0}")]
    Unexpected(String),
}

impl From<reqwest::Error> for RestError {
    fn from(e: reqwest::Error) -> Self {
        // `without_url` keeps the cache-buster query noise out of error text.
        RestError::Http(e.without_url().to_string())
    }
}

impl From<serde_json::Error> for RestError {
    fn from(e: serde_json::Error) -> Self {
        RestError::Json(e.to_string())
    }
}

/// Base URLs of the three endpoints. Tests point them at a local mock server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestUrls {
    pub gamma_base: String,
    pub data_base: String,
    pub geoblock_url: String,
}

impl Default for RestUrls {
    fn default() -> Self {
        RestUrls {
            gamma_base: "https://gamma-api.polymarket.com".into(),
            data_base: "https://data-api.polymarket.com".into(),
            geoblock_url: "https://polymarket.com/api/geoblock".into(),
        }
    }
}

/// Client for the public Polymarket REST endpoints.
pub struct PolyRest {
    http: reqwest::Client,
    gamma_base: String,
    data_base: String,
    geoblock_url: String,
    rec: Option<RecordTx>,
}

/// A fetched response: status and the raw body text.
pub(crate) struct Response {
    pub status: u16,
    pub body: String,
}

impl PolyRest {
    /// No `User-Agent` is set on purpose: CLOB answered 403 to library-looking agents.
    pub fn new(urls: RestUrls, rec: Option<RecordTx>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client with static config");
        PolyRest {
            http,
            gamma_base: urls.gamma_base.trim_end_matches('/').to_owned(),
            data_base: urls.data_base.trim_end_matches('/').to_owned(),
            geoblock_url: urls.geoblock_url,
            rec,
        }
    }

    pub(crate) fn gamma_base(&self) -> &str {
        &self.gamma_base
    }

    pub(crate) fn data_base(&self) -> &str {
        &self.data_base
    }

    pub(crate) fn geoblock_url(&self) -> &str {
        &self.geoblock_url
    }

    pub(crate) async fn get(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<(String, Response), RestError> {
        let resp = req.send().await?;
        let url = resp.url().to_string();
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        Ok((url, Response { status, body }))
    }

    /// Records one HTTP response. `body` is the JSON value stored under `"body"`.
    pub(crate) fn record_http(
        &self,
        source: Source,
        url: &str,
        status: u16,
        body: serde_json::Value,
    ) {
        let Some(rec) = &self.rec else { return };
        rec.send(Record {
            t: now_ns(),
            s: source,
            c: 0,
            k: RecordKind::Http,
            d: json!({"url": url, "status": status, "body": body}).to_string(),
        });
    }
}
