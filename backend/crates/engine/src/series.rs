//! The latest prices for one window, shared by the per-second series and the live view.

use std::collections::HashMap;

use raki_core::{Asset, OracleKind, SeriesRow};
use rust_decimal::Decimal;

use crate::book::BookState;
use crate::cex::CexState;
use crate::oracle::OracleState;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowPrices {
    /// `(ts_ms, value)` of the newest TWAP point.
    pub twap: Option<(i64, f64)>,
    pub chainlink: Option<f64>,
    pub binance_mid_usd: Option<f64>,
    pub coinbase_mid: Option<f64>,
    pub up_bid: Option<Decimal>,
    pub up_ask: Option<Decimal>,
    pub down_bid: Option<Decimal>,
    pub down_ask: Option<Decimal>,
}

impl WindowPrices {
    /// Reads the current state. Book prices are `None` while a token is unknown or has no book.
    pub fn collect(
        asset: Asset,
        up_token: Option<&str>,
        down_token: Option<&str>,
        oracle: &OracleState,
        cex: &CexState,
        books: &HashMap<String, BookState>,
    ) -> WindowPrices {
        let book = |token: Option<&str>| token.and_then(|t| books.get(t));
        let (up, down) = (book(up_token), book(down_token));
        WindowPrices {
            twap: oracle
                .latest(asset, OracleKind::Twap60)
                .map(|(ts, value, _)| (ts, value)),
            chainlink: oracle
                .latest(asset, OracleKind::Chainlink)
                .map(|(_, value, _)| value),
            binance_mid_usd: cex.binance_mid_usd(asset),
            coinbase_mid: cex.coinbase_mid(asset),
            up_bid: up.and_then(BookState::best_bid),
            up_ask: up.and_then(BookState::best_ask),
            down_bid: down.and_then(BookState::best_bid),
            down_ask: down.and_then(BookState::best_ask),
        }
    }

    pub fn series_row(&self, slug: &str, ts: i64) -> SeriesRow {
        SeriesRow {
            slug: slug.to_string(),
            ts,
            twap: self.twap.map(|(_, value)| value),
            chainlink: self.chainlink,
            binance_mid_usd: self.binance_mid_usd,
            coinbase_mid: self.coinbase_mid,
            up_bid: self.up_bid,
            up_ask: self.up_ask,
            down_bid: self.down_bid,
            down_ask: self.down_ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use raki_core::{Level, Source, Venue};

    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn collect_reads_latest_prices_and_books() {
        let mut oracle = OracleState::default();
        oracle.insert(
            Asset::Btc,
            OracleKind::Twap60,
            Source::Polybolt,
            1_000,
            10.0,
        );
        oracle.insert(
            Asset::Btc,
            OracleKind::Twap60,
            Source::Polybolt,
            2_000,
            11.0,
        );
        oracle.insert(
            Asset::Btc,
            OracleKind::Chainlink,
            Source::Polybolt,
            2_000,
            12.0,
        );
        let mut cex = CexState::default();
        cex.on_quote(Venue::Coinbase, Asset::Btc, d("99"), d("101"), 0);
        let mut books = HashMap::new();
        let mut up = BookState::default();
        up.apply_snapshot(
            &[Level {
                price: d("0.6"),
                size: d("5"),
            }],
            &[Level {
                price: d("0.62"),
                size: d("5"),
            }],
        );
        books.insert("UP".to_string(), up);

        let p = WindowPrices::collect(Asset::Btc, Some("UP"), Some("DOWN"), &oracle, &cex, &books);
        assert_eq!(p.twap, Some((2_000, 11.0)));
        assert_eq!(p.chainlink, Some(12.0));
        assert_eq!(p.binance_mid_usd, None);
        assert_eq!(p.coinbase_mid, Some(100.0));
        assert_eq!((p.up_bid, p.up_ask), (Some(d("0.6")), Some(d("0.62"))));
        assert_eq!((p.down_bid, p.down_ask), (None, None));

        let row = p.series_row("btc-updown-5m-1793031000", 1793031001);
        assert_eq!(row.slug, "btc-updown-5m-1793031000");
        assert_eq!(row.ts, 1793031001);
        assert_eq!(row.twap, Some(11.0));
        assert_eq!(row.up_ask, Some(d("0.62")));

        let none = WindowPrices::collect(Asset::Eth, None, None, &oracle, &cex, &books);
        assert_eq!(none, WindowPrices::default());
    }
}
