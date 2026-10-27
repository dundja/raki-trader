use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Asset, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

/// Market outcome; serialized as `"Up"` / `"Down"`, matching Gamma's outcome strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum Outcome {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OracleKind {
    Chainlink,
    Twap60,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    Binance,
    Coinbase,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PriceChangeEntry {
    pub asset_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub side: Side,
    pub best_bid: Option<Decimal>,
    pub best_ask: Option<Decimal>,
}

/// The fields of a Gamma 5-minute event that the recorder cares about.
#[derive(Clone, Debug, PartialEq)]
pub struct GammaWindow {
    pub slug: String,
    pub event_id: String,
    pub start: i64,
    pub end: i64,
    pub condition_id: String,
    pub up_token: String,
    pub down_token: String,
    pub price_to_beat: Option<f64>,
    pub final_price: Option<f64>,
    pub closed: bool,
    pub twap_lookback_s: Option<i64>,
    pub fee_rate: Option<f64>,
    pub tick_size: Option<f64>,
}

/// A parsed message from any feed or REST poll, as consumed by the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedEvent {
    Book {
        asset_id: String,
        market: String,
        ts_ms: i64,
        bids: Vec<Level>,
        asks: Vec<Level>,
    },
    PriceChange {
        market: String,
        ts_ms: i64,
        changes: Vec<PriceChangeEntry>,
    },
    BestBidAsk {
        asset_id: String,
        ts_ms: i64,
        best_bid: Option<Decimal>,
        best_ask: Option<Decimal>,
    },
    LastTrade {
        asset_id: String,
        ts_ms: i64,
        price: Decimal,
        size: Decimal,
        side: Side,
    },
    TickSize {
        asset_id: String,
        ts_ms: i64,
        new_tick: Decimal,
    },
    MarketResolved {
        condition_id: String,
        winning_asset_id: String,
        winning_outcome: String,
        recv_ms: i64,
    },
    Oracle {
        asset: Asset,
        kind: OracleKind,
        source: Source,
        ts_ms: i64,
        value: f64,
    },
    CexQuote {
        venue: Venue,
        asset: Asset,
        bid: Decimal,
        ask: Decimal,
        recv_ms: i64,
    },
    UsdtUsd {
        price: Decimal,
        recv_ms: i64,
    },
    /// Emitted by a runner on every (re)connect. `shard` names the asset of a feed that runs one
    /// connection per asset (the CLOB); `None` for unsharded feeds.
    FeedReset {
        source: Source,
        shard: Option<Asset>,
    },
    /// `window: None` means Gamma answered 404.
    GammaFetched {
        slug: String,
        window: Option<GammaWindow>,
        recv_ms: i64,
    },
    GammaFailed {
        slug: String,
    },
    /// `winner: None` means not resolved yet.
    Resolution {
        condition_id: String,
        winner: Option<Outcome>,
        resolved_at: Option<i64>,
    },
}
