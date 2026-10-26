//! Latest Binance and Coinbase top-of-book mids, plus USDT/USD for converting Binance prices.

use std::collections::BTreeMap;

use raki_core::{Asset, Venue};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Default)]
pub struct CexState {
    binance_mid: BTreeMap<Asset, Decimal>,
    coinbase_mid: BTreeMap<Asset, Decimal>,
    usdt_usd: Option<Decimal>,
}

impl CexState {
    /// Records the latest bid/ask. `_recv_ms` is part of the feed contract but no M1 consumer
    /// reads quote age (feed freshness is tracked per feed by the shell).
    pub fn on_quote(
        &mut self,
        venue: Venue,
        asset: Asset,
        bid: Decimal,
        ask: Decimal,
        _recv_ms: i64,
    ) {
        let mids = match venue {
            Venue::Binance => &mut self.binance_mid,
            Venue::Coinbase => &mut self.coinbase_mid,
        };
        mids.insert(asset, (bid + ask) / Decimal::TWO);
    }

    pub fn on_usdt(&mut self, price: Decimal, _recv_ms: i64) {
        self.usdt_usd = Some(price);
    }

    /// Binance mid (quoted in USDT) times USDT/USD. `None` until both are known.
    pub fn binance_mid_usd(&self, asset: Asset) -> Option<f64> {
        (self.binance_mid.get(&asset)? * self.usdt_usd?).to_f64()
    }

    pub fn coinbase_mid(&self, asset: Asset) -> Option<f64> {
        self.coinbase_mid.get(&asset)?.to_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn approx(got: Option<f64>, want: f64) {
        let got = got.expect("a value");
        assert!((got - want).abs() < 1e-9, "{got} != {want}");
    }

    #[test]
    fn binance_mid_needs_usdt_and_is_converted() {
        let mut c = CexState::default();
        c.on_quote(Venue::Binance, Asset::Btc, d("100"), d("102"), 1);
        assert_eq!(c.binance_mid_usd(Asset::Btc), None);
        c.on_usdt(d("0.999"), 2);
        approx(c.binance_mid_usd(Asset::Btc), 100.899);
        assert_eq!(c.coinbase_mid(Asset::Btc), None);
        assert_eq!(c.binance_mid_usd(Asset::Eth), None);
    }

    #[test]
    fn coinbase_mid_is_plain_mid() {
        let mut c = CexState::default();
        c.on_quote(Venue::Coinbase, Asset::Sol, d("10.01"), d("10.02"), 1);
        approx(c.coinbase_mid(Asset::Sol), 10.015);
        assert_eq!(c.binance_mid_usd(Asset::Sol), None);
    }
}
