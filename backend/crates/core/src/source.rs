use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where a recorded record or event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    ClobMarket,
    RtdsLegacy,
    Polybolt,
    Binance,
    Coinbase,
    Gamma,
    DataApi,
    Geoblock,
    System,
}

impl Source {
    /// Stable name; identical to the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::ClobMarket => "clob_market",
            Source::RtdsLegacy => "rtds_legacy",
            Source::Polybolt => "polybolt",
            Source::Binance => "binance",
            Source::Coinbase => "coinbase",
            Source::Gamma => "gamma",
            Source::DataApi => "data_api",
            Source::Geoblock => "geoblock",
            Source::System => "system",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_serde_name() {
        for s in [
            Source::ClobMarket,
            Source::RtdsLegacy,
            Source::Polybolt,
            Source::Binance,
            Source::Coinbase,
            Source::Gamma,
            Source::DataApi,
            Source::Geoblock,
            Source::System,
        ] {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
    }
}
