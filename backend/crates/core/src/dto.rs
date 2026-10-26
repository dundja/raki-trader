//! Every type the HTTP and WebSocket API returns or accepts.
//!
//! These derive [`ts_rs::TS`] and are exported to `frontend/src/api/generated` by the
//! `export_bindings_*` tests. `i64`/`u64` are typed `number` (JSON cannot carry bigint) and
//! `Decimal` is a string on the wire.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Asset, Outcome, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowStatus {
    Upcoming,
    Live,
    Ended,
    Resolved,
    Complete,
    TimedOut,
}

/// One tracked window, as stored in Postgres. `*_ours` values come from our own recording;
/// `*_gamma` values come from Polymarket's Gamma API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WindowRecord {
    pub slug: String,
    pub asset: Asset,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub end_ts: i64,
    pub event_id: Option<String>,
    pub condition_id: Option<String>,
    pub up_token: Option<String>,
    pub down_token: Option<String>,
    pub price_to_beat_ours: Option<f64>,
    pub final_twap_ours: Option<f64>,
    pub outcome_ours: Option<Outcome>,
    pub price_to_beat_gamma: Option<f64>,
    pub final_price_gamma: Option<f64>,
    pub twap_match: Option<bool>,
    pub winner: Option<Outcome>,
    #[ts(type = "number | null")]
    pub resolved_at: Option<i64>,
    #[ts(type = "number | null")]
    pub resolution_seen_ms: Option<i64>,
    pub twap_source: Option<Source>,
    pub coverage_secs: i32,
    pub status: WindowStatus,
    pub session_id: String,
    #[ts(type = "number")]
    pub updated_ms: i64,
}

/// One per-second sample of a window's series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SeriesRow {
    pub slug: String,
    #[ts(type = "number")]
    pub ts: i64,
    pub twap: Option<f64>,
    pub chainlink: Option<f64>,
    pub binance_mid_usd: Option<f64>,
    pub coinbase_mid: Option<f64>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub up_bid: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub up_ask: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub down_bid: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub down_ask: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedStatusDto {
    pub source: Source,
    pub enabled: bool,
    pub connected: bool,
    #[ts(type = "number | null")]
    pub last_msg_ms: Option<i64>,
    #[ts(type = "number")]
    pub msgs_total: u64,
    pub msgs_per_sec: f64,
    pub reconnects: u32,
    pub last_error: Option<String>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClockStatus {
    pub offset_ms: Option<f64>,
    pub ok: bool,
    #[ts(type = "number | null")]
    pub checked_ms: Option<i64>,
    pub max_offset_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GeoStatus {
    pub blocked: Option<bool>,
    pub country: Option<String>,
    #[ts(type = "number | null")]
    pub checked_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecorderStatus {
    pub file: Option<String>,
    #[ts(type = "number")]
    pub records: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub dropped: u64,
    #[ts(type = "number | null")]
    pub last_flush_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SafetyChecks {
    pub feeds_fresh: bool,
    pub clock_ok: bool,
    pub geoblock_ok: bool,
    pub recorder_ok: bool,
    pub all_ok: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthSnapshot {
    pub feeds: Vec<FeedStatusDto>,
    pub clock: ClockStatus,
    pub geoblock: GeoStatus,
    pub recorder: RecorderStatus,
    pub sleep_events: u32,
    pub book_resyncs: u32,
    pub safety: SafetyChecks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TradingMode {
    Running,
    Paused,
    Killed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopKind {
    Startup,
    Manual,
    Safety,
    LossCap,
    ConfigHalt,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ControlState {
    pub mode: TradingMode,
    pub stop_kind: Option<StopKind>,
    pub reason: Option<String>,
    pub needs_ack: bool,
    #[ts(type = "number")]
    pub since_ms: i64,
}

/// A currently open window with the latest known prices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveWindow {
    pub asset: Asset,
    pub slug: String,
    #[ts(type = "number")]
    pub start_ts: i64,
    #[ts(type = "number")]
    pub end_ts: i64,
    pub secs_left: f64,
    pub price_to_beat: Option<f64>,
    pub twap: Option<f64>,
    #[ts(type = "number | null")]
    pub twap_ts_ms: Option<i64>,
    pub chainlink: Option<f64>,
    pub binance_mid_usd: Option<f64>,
    pub coinbase_mid: Option<f64>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub up_bid: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub up_ask: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub down_bid: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub down_ask: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionInfo {
    pub id: String,
    #[ts(type = "number")]
    pub started_ms: i64,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveSnapshot {
    #[ts(type = "number")]
    pub generated_ms: i64,
    pub session: SessionInfo,
    pub health: HealthSnapshot,
    pub control: ControlState,
    pub windows: Vec<LiveWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecordingStats {
    #[ts(type = "number")]
    pub sessions: i64,
    #[ts(type = "number")]
    pub recorded_secs: i64,
    #[ts(type = "number")]
    pub windows_total: i64,
    #[ts(type = "number")]
    pub windows_with_twap: i64,
    #[ts(type = "number")]
    pub twap_match: i64,
    #[ts(type = "number")]
    pub twap_mismatch: i64,
    #[ts(type = "number")]
    pub twap_pending: i64,
    /// Number of windows needed to pass the M1 gate (500).
    #[ts(type = "number")]
    pub gate_target_windows: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ControlRequest {
    pub reason: Option<String>,
    pub acknowledge: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ControlResponse {
    pub ok: bool,
    pub state: ControlState,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_strings_on_the_wire() {
        let row = SeriesRow {
            slug: "btc-updown-5m-1793031000".into(),
            ts: 1793031001,
            twap: Some(1.5),
            chainlink: None,
            binance_mid_usd: None,
            coinbase_mid: None,
            up_bid: Some("0.52".parse().unwrap()),
            up_ask: None,
            down_bid: None,
            down_ask: Some("0.48".parse().unwrap()),
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["up_bid"], "0.52");
        assert!(v["up_ask"].is_null());
        assert_eq!(serde_json::from_value::<SeriesRow>(v).unwrap(), row);
    }
}
