use serde::{Deserialize, Serialize};

use crate::Asset;

/// Length of one up/down window in seconds.
pub const WINDOW_SECS: i64 = 300;

const SLUG_INFIX: &str = "-updown-5m-";

/// One 5-minute market window for one asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId {
    pub asset: Asset,
    /// Unix seconds, always a multiple of [`WINDOW_SECS`].
    pub start: i64,
}

impl WindowId {
    /// The window that contains `unix_s`.
    pub fn containing(asset: Asset, unix_s: i64) -> WindowId {
        WindowId {
            asset,
            start: unix_s.div_euclid(WINDOW_SECS) * WINDOW_SECS,
        }
    }

    pub fn next(self) -> WindowId {
        WindowId {
            asset: self.asset,
            start: self.start + WINDOW_SECS,
        }
    }

    pub fn end(self) -> i64 {
        self.start + WINDOW_SECS
    }

    /// Gamma event slug, e.g. `btc-updown-5m-1793031000`.
    pub fn slug(self) -> String {
        format!("{}{SLUG_INFIX}{}", self.asset.as_str(), self.start)
    }

    /// Inverse of [`slug`](Self::slug). Rejects unknown assets, other durations and
    /// start times that are not a multiple of 300.
    pub fn parse_slug(s: &str) -> Option<WindowId> {
        let (asset, start) = s.split_once(SLUG_INFIX)?;
        let asset = Asset::ALL.into_iter().find(|a| a.as_str() == asset)?;
        let start: i64 = start.parse().ok()?;
        (start % WINDOW_SECS == 0).then_some(WindowId { asset, start })
    }

    /// Seconds until the window ends, measured from `now_ms`. Negative once ended.
    pub fn secs_left(self, now_ms: i64) -> f64 {
        (self.end() * 1000 - now_ms) as f64 / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use crate::{Asset, WindowId};

    #[test]
    fn containing_floors_to_300() {
        assert_eq!(
            WindowId::containing(Asset::Btc, 1793031299).start,
            1793031000
        );
        assert_eq!(
            WindowId::containing(Asset::Btc, 1793031300).start,
            1793031300
        );
    }

    #[test]
    fn slug_roundtrip() {
        let id = WindowId {
            asset: Asset::Btc,
            start: 1793031000,
        };
        assert_eq!(id.slug(), "btc-updown-5m-1793031000");
        assert_eq!(WindowId::parse_slug("btc-updown-5m-1793031000"), Some(id));
    }

    #[test]
    fn parse_slug_rejects() {
        for s in [
            "btc-updown-5m-1793031001",
            "doge-updown-5m-1793031000",
            "btc-updown-15m-1793031000",
            "garbage",
        ] {
            assert_eq!(WindowId::parse_slug(s), None, "{s}");
        }
    }

    #[test]
    fn secs_left_math() {
        let id = WindowId {
            asset: Asset::Btc,
            start: 1793031000,
        };
        assert_eq!(id.secs_left(1793031270000), 30.0);
        assert_eq!(id.next().start, 1793031300);
        assert_eq!(id.end(), 1793031300);
    }

    #[test]
    fn asset_symbols() {
        assert_eq!(Asset::Btc.rtds_symbol(), "btc/usd");
        assert_eq!(Asset::Btc.polybolt_symbol(), "btcusd");
        assert_eq!(Asset::Btc.coinbase_product(), "BTC-USD");
        assert_eq!(Asset::from_any("BTCUSDT"), Some(Asset::Btc));
        assert_eq!(Asset::from_any("sol/usd"), Some(Asset::Sol));
    }
}
