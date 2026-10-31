//! One CLOB order book (a single outcome token), rebuilt from snapshots and deltas.

use std::collections::BTreeMap;

use raki_core::{Level, Side};
use rust_decimal::Decimal;

/// Price → size per side. A level with size 0 is never stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BookState {
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
}

impl BookState {
    /// Replaces the whole book with a `book` snapshot. Returns `true` if the snapshot differed
    /// from the book we had built (a resync); comparing a server snapshot with our delta-built
    /// book is how drift is detected.
    pub fn apply_snapshot(&mut self, bids: &[Level], asks: &[Level]) -> bool {
        let snapshot = BookState {
            bids: to_levels(bids),
            asks: to_levels(asks),
        };
        let differed = *self != snapshot;
        *self = snapshot;
        differed
    }

    /// Applies one `price_change` entry: BUY → bids, SELL → asks; size 0 removes the level.
    pub fn apply_change(&mut self, side: Side, price: Decimal, size: Decimal) {
        let levels = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        if size.is_zero() {
            levels.remove(&price);
        } else {
            levels.insert(price, size);
        }
    }

    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.last_key_value().map(|(p, _)| *p)
    }

    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first_key_value().map(|(p, _)| *p)
    }

    /// (price, size), ascending by price.
    pub fn asks(&self) -> Vec<(Decimal, Decimal)> {
        self.asks.iter().map(|(p, s)| (*p, *s)).collect()
    }
}

fn to_levels(levels: &[Level]) -> BTreeMap<Decimal, Decimal> {
    levels
        .iter()
        .filter(|l| !l.size.is_zero())
        .map(|l| (l.price, l.size))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use raki_core::FeedEvent;
    use raki_feeds::parse::clob::parse_clob;

    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn lv(price: &str, size: &str) -> Level {
        Level {
            price: d(price),
            size: d(size),
        }
    }

    fn fixture_events(name: &str) -> Vec<FeedEvent> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        text.lines()
            .flat_map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                parse_clob(v["raw"].as_str().unwrap(), v["recv_ms"].as_i64().unwrap()).unwrap()
            })
            .collect()
    }

    #[test]
    fn book_replay_matches_checkpoints() {
        let mut books: HashMap<String, BookState> = HashMap::new();
        let mut last_book_best: HashMap<String, (Option<Decimal>, Option<Decimal>)> =
            HashMap::new();
        let (mut checkpoints, mut matched) = (0, 0);
        for ev in fixture_events("clob/live_window.jsonl") {
            match ev {
                FeedEvent::Book {
                    asset_id,
                    bids,
                    asks,
                    ..
                } => {
                    let had = books.contains_key(&asset_id);
                    let book = books.entry(asset_id.clone()).or_default();
                    let differed = book.apply_snapshot(&bids, &asks);
                    if had {
                        checkpoints += 1;
                        if !differed {
                            matched += 1;
                        }
                    }
                    last_book_best.insert(asset_id, (book.best_bid(), book.best_ask()));
                }
                FeedEvent::PriceChange { changes, .. } => {
                    for c in changes {
                        books
                            .get_mut(&c.asset_id)
                            .expect("price_change before its book snapshot")
                            .apply_change(c.side, c.price, c.size);
                    }
                }
                _ => {}
            }
        }
        assert!(checkpoints >= 20, "only {checkpoints} checkpoints");
        assert!(
            matched * 100 >= checkpoints * 95,
            "{matched}/{checkpoints} checkpoints matched"
        );
        assert!(!books.is_empty());
        for (asset_id, book) in &books {
            assert_eq!(
                (book.best_bid(), book.best_ask()),
                last_book_best[asset_id],
                "{asset_id}"
            );
        }
    }

    #[test]
    fn book_size_zero_removes_level() {
        let mut book = BookState::default();
        assert!(book.apply_snapshot(
            &[lv("0.50", "10"), lv("0.49", "5")],
            &[lv("0.52", "7"), lv("0.53", "0")]
        ));
        assert_eq!(book.best_bid(), Some(d("0.50")));
        assert_eq!(book.best_ask(), Some(d("0.52")));

        book.apply_change(Side::Buy, d("0.50"), d("0"));
        book.apply_change(Side::Sell, d("0.52"), d("0"));
        assert_eq!(book.best_bid(), Some(d("0.49")));
        assert_eq!(
            book.best_ask(),
            None,
            "the size-0 snapshot level was never stored"
        );

        book.apply_change(Side::Sell, d("0.51"), d("3"));
        assert_eq!(book.best_ask(), Some(d("0.51")));
        assert!(!book.apply_snapshot(&[lv("0.49", "5")], &[lv("0.51", "3")]));
        assert!(book.apply_snapshot(&[lv("0.49", "6")], &[lv("0.51", "3")]));
    }
}
