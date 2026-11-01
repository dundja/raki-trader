//! Shadow predictions and strategy v0 shadow decisions (docs/specs/2026-10-31-shadow-predictions.md):
//! the rows the engine emits and stores, and what the API returns about them.
//!
//! Shadow results never count toward the M2 or M3 gates.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Asset, Outcome};

/// Seconds before the window end at which a prediction row is stored.
pub const PREDICTION_CHECKPOINTS: [i64; 12] = [240, 180, 120, 90, 60, 45, 40, 35, 30, 20, 10, 5];

/// The model's forecast for one window at one checkpoint (seconds before the end).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PredictionRow {
    pub slug: String,
    pub asset: Asset,
    #[ts(type = "number")]
    pub checkpoint_s: i64,
    #[ts(type = "number")]
    pub ts_ms: i64,
    pub p_up: f64,
    /// Mean and standard deviation of the forecast final TWAP, and the per-second σ used.
    pub mu: f64,
    pub sd: f64,
    pub sigma: f64,
    pub ptb: f64,
    pub spot: f64,
    pub twap: Option<f64>,
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
    pub model_id: String,
    pub session_id: String,
}

/// The latest forecast of a live window, pushed with the live snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LivePrediction {
    pub p_up: f64,
    pub mu: f64,
    pub sd: f64,
    /// Timestamp of the spot print the forecast starts from.
    #[ts(type = "number")]
    pub spot_ts_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ShadowAction {
    Enter,
    Skip,
}

/// One strategy evaluation (one per tick in the entry window). `seq` is unique per session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShadowEvalRow {
    pub session_id: String,
    #[ts(type = "number")]
    pub seq: i64,
    pub strategy_id: String,
    pub params_hash: String,
    pub slug: String,
    #[ts(type = "number")]
    pub ts_ms: i64,
    #[ts(type = "number")]
    pub secs_left: i64,
    pub side: Option<Outcome>,
    pub model_prob: Option<f64>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub market_prob: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub best_ask: Option<Decimal>,
    /// What a buy of the stake at or below `max_entry_price` would have taken.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub walk_shares: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub walk_avg_price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub walk_fee: Option<Decimal>,
    /// `model_prob − all-in price per share`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub edge: Option<Decimal>,
    pub action: ShadowAction,
    /// Why it skipped (`unsafe`, `no_prediction`, `market_config`, `no_book`, `no_favourite`,
    /// `no_liquidity`, `edge`; `not_running` in rows from before the shadow ignored pauses).
    pub reason: Option<String>,
    /// Control was `running` (a live bot would have traded); false while paused.
    pub control_running: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ShadowFillStatus {
    Filled,
    Partial,
    NoFill,
}

/// The simulated fill of an `enter` evaluation, against the book `sim_latency_ms` later.
/// `winner` and `pnl` are filled in when read back once the window resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShadowFillRow {
    pub session_id: String,
    #[ts(type = "number")]
    pub seq: i64,
    pub slug: String,
    pub side: Outcome,
    #[ts(type = "number")]
    pub decided_ms: i64,
    #[ts(type = "number")]
    pub due_ms: i64,
    #[ts(type = "number")]
    pub filled_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub limit_price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub shares: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub cost: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub fee: Decimal,
    pub status: ShadowFillStatus,
    pub winner: Option<Outcome>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[ts(type = "string | null")]
    pub pnl: Option<Decimal>,
}

/// v0's state in a live window, for the live card.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveShadow {
    pub last_eval: Option<ShadowEvalRow>,
    pub fill: Option<ShadowFillRow>,
    /// An `enter` waiting for its simulated fill.
    pub pending: bool,
}

/// A window's v0 shadow: every evaluation and every simulated fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WindowShadow {
    pub evals: Vec<ShadowEvalRow>,
    pub fills: Vec<ShadowFillRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReasonCount {
    pub reason: String,
    #[ts(type = "number")]
    pub count: i64,
}

/// Everything v0's shadow did so far, all sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShadowSummary {
    #[ts(type = "number")]
    pub evaluations: i64,
    /// Of those, made while control was paused (safety checks passing again).
    #[ts(type = "number")]
    pub evaluations_while_paused: i64,
    #[ts(type = "number")]
    pub windows_evaluated: i64,
    /// Windows with at least one `enter`.
    #[ts(type = "number")]
    pub windows_entered: i64,
    #[ts(type = "number")]
    pub entries: i64,
    #[ts(type = "number")]
    pub filled: i64,
    #[ts(type = "number")]
    pub partial: i64,
    #[ts(type = "number")]
    pub no_fill: i64,
    /// Fills (filled or partial) whose window has a winner.
    #[ts(type = "number")]
    pub resolved: i64,
    #[ts(type = "number")]
    pub wins: i64,
    #[ts(type = "number")]
    pub losses: i64,
    /// Over resolved fills: dollars paid (fee included) and net profit.
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub staked: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[ts(type = "string")]
    pub pnl: Decimal,
    /// Mean edge of the `enter` evaluations that filled, as predicted.
    pub mean_predicted_edge: Option<f64>,
    /// Skip reasons over all evaluations, most frequent first.
    pub skip_reasons: Vec<ReasonCount>,
    pub params_hashes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CalibrationBucket {
    pub label: String,
    pub lo: f64,
    pub hi: f64,
    #[ts(type = "number")]
    pub n: i64,
    /// Mean predicted probability of the favourite, and how often it won.
    pub mean_predicted: Option<f64>,
    pub realized: Option<f64>,
    /// Wilson 95% interval of `realized`.
    pub ci_lo: Option<f64>,
    pub ci_hi: Option<f64>,
}

/// Predicted vs realized at one checkpoint, over resolved windows. The favourite is the side
/// the model gives at least 0.5.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Calibration {
    #[ts(type = "number")]
    pub checkpoint_s: i64,
    pub asset: Option<Asset>,
    #[ts(type = "number")]
    pub n: i64,
    /// Mean squared error of `p_up` against the Up outcome (lower is better; 0.25 is a coin).
    pub brier: Option<f64>,
    pub buckets: Vec<CalibrationBucket>,
}
