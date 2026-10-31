//! Walking an ask book for a taker buy of a fixed dollar stake, and the taker fee.

use rust_decimal::Decimal;

/// What a taker buy of up to `stake` dollars at or below `limit` would take from the asks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Walk {
    pub shares: Decimal,
    /// Dollars paid for the shares, fee excluded.
    pub cost: Decimal,
    /// Taker fee in dollars, summed per level.
    pub fee: Decimal,
    /// The highest price level used.
    pub worst_price: Option<Decimal>,
    /// The whole stake was spent (the book had enough at or below the limit).
    pub stake_spent: bool,
}

impl Walk {
    pub fn avg_price(&self) -> Option<Decimal> {
        (!self.shares.is_zero()).then(|| self.cost / self.shares)
    }

    /// Price plus fee per share: what one share really costs.
    pub fn all_in_per_share(&self) -> Option<Decimal> {
        (!self.shares.is_zero()).then(|| (self.cost + self.fee) / self.shares)
    }
}

/// Polymarket's taker fee: `shares · rate · p · (1 − p)` (fee schedule exponent 1). Unrounded:
/// the platform's rounding is unverified until a real fill (M4).
pub fn taker_fee(shares: Decimal, price: Decimal, rate: Decimal) -> Decimal {
    shares * rate * price * (Decimal::ONE - price)
}

/// Buys from `asks` (price, size), ascending by price, until `stake` dollars are spent or the
/// next level is above `limit`. The last level may be taken partly.
pub fn walk_asks(
    asks: impl IntoIterator<Item = (Decimal, Decimal)>,
    limit: Decimal,
    stake: Decimal,
    fee_rate: Decimal,
) -> Walk {
    let mut w = Walk::default();
    if stake <= Decimal::ZERO {
        return w;
    }
    for (price, size) in asks {
        if price > limit || price <= Decimal::ZERO {
            break;
        }
        let left = stake - w.cost;
        let affordable = left / price;
        let (take, spent) = if affordable <= size {
            (affordable, true)
        } else {
            (size, false)
        };
        w.shares += take;
        // The last level costs exactly what was left, so no rounding dust remains.
        w.cost = if spent { stake } else { w.cost + take * price };
        w.fee += taker_fee(take, price, fee_rate);
        w.worst_price = Some(price);
        if spent {
            w.stake_spent = true;
            break;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn book(levels: &[(&str, &str)]) -> Vec<(Decimal, Decimal)> {
        levels.iter().map(|(p, s)| (d(p), d(s))).collect()
    }

    #[test]
    fn fee_formula() {
        // 10 shares at 0.92: 10 · 0.07 · 0.92 · 0.08 = 0.05152.
        assert_eq!(taker_fee(d("10"), d("0.92"), d("0.07")), d("0.05152"));
        assert_eq!(taker_fee(d("10"), d("1"), d("0.07")), d("0"));
    }

    #[test]
    fn stake_spread_over_levels_with_a_partial_last_level() {
        // 3 at 0.93 cost 2.79; the remaining 2.21 buys 2.3263... of the 100 at 0.95.
        let w = walk_asks(
            book(&[("0.93", "3"), ("0.95", "100"), ("0.96", "100")]),
            d("0.97"),
            d("5"),
            d("0.07"),
        );
        let second = d("2.21") / d("0.95");
        assert_eq!(w.shares, d("3") + second);
        assert_eq!(w.cost, d("5"));
        assert!(w.stake_spent);
        assert_eq!(w.worst_price, Some(d("0.95")));
        let fee = taker_fee(d("3"), d("0.93"), d("0.07")) + taker_fee(second, d("0.95"), d("0.07"));
        assert_eq!(w.fee, fee);
        assert_eq!(w.avg_price(), Some(d("5") / w.shares));
        assert_eq!(w.all_in_per_share(), Some((d("5") + fee) / w.shares));
    }

    #[test]
    fn limit_cuts_the_walk() {
        let w = walk_asks(
            book(&[("0.93", "2"), ("0.98", "100")]),
            d("0.97"),
            d("5"),
            d("0.07"),
        );
        assert_eq!((w.shares, w.cost), (d("2"), d("1.86")));
        assert!(!w.stake_spent);
        assert_eq!(w.worst_price, Some(d("0.93")));
    }

    #[test]
    fn empty_or_too_expensive_book_buys_nothing() {
        for asks in [book(&[]), book(&[("0.99", "500")])] {
            let w = walk_asks(asks, d("0.97"), d("5"), d("0.07"));
            assert_eq!(w, Walk::default());
            assert_eq!(w.avg_price(), None);
        }
        assert_eq!(
            walk_asks(book(&[("0.9", "9")]), d("0.97"), d("0"), d("0.07")),
            Walk::default()
        );
    }
}
