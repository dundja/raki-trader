use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The three tracked crypto assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Asset {
    Btc,
    Eth,
    Sol,
}

impl Asset {
    pub const ALL: [Asset; 3] = [Asset::Btc, Asset::Eth, Asset::Sol];

    /// Lowercase ticker, as used in market slugs: `"btc"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Asset::Btc => "btc",
            Asset::Eth => "eth",
            Asset::Sol => "sol",
        }
    }

    /// Polymarket RTDS legacy symbol: `"btc/usd"`.
    pub fn rtds_symbol(self) -> &'static str {
        match self {
            Asset::Btc => "btc/usd",
            Asset::Eth => "eth/usd",
            Asset::Sol => "sol/usd",
        }
    }

    /// PolyBolt symbol: `"btcusd"`.
    pub fn polybolt_symbol(self) -> &'static str {
        match self {
            Asset::Btc => "btcusd",
            Asset::Eth => "ethusd",
            Asset::Sol => "solusd",
        }
    }

    /// Binance stream name (lowercase): `"btcusdt"`.
    pub fn binance_symbol(self) -> &'static str {
        match self {
            Asset::Btc => "btcusdt",
            Asset::Eth => "ethusdt",
            Asset::Sol => "solusdt",
        }
    }

    /// Coinbase product id: `"BTC-USD"`.
    pub fn coinbase_product(self) -> &'static str {
        match self {
            Asset::Btc => "BTC-USD",
            Asset::Eth => "ETH-USD",
            Asset::Sol => "SOL-USD",
        }
    }

    /// Accepts every symbol form above (and the bare ticker), case-insensitively.
    pub fn from_any(s: &str) -> Option<Asset> {
        let s = s.to_ascii_lowercase();
        Asset::ALL.into_iter().find(|a| {
            s == a.as_str()
                || s == a.rtds_symbol()
                || s == a.polybolt_symbol()
                || s == a.binance_symbol()
                || s == a.coinbase_product().to_ascii_lowercase()
        })
    }
}
