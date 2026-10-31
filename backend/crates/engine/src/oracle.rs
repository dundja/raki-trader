//! Recent Chainlink price and TWAP points per asset, keyed by source timestamp.

use std::collections::BTreeMap;

use raki_core::{Asset, OracleKind, Source};

/// How long points are kept, measured back from `now_ms` in [`OracleState::prune`].
pub const RETENTION_MS: i64 = 15 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    value: f64,
    source: Source,
}

/// The last [`RETENTION_MS`] of points per (asset, kind).
#[derive(Debug, Default)]
pub struct OracleState {
    series: BTreeMap<(Asset, usize), BTreeMap<i64, Point>>,
}

impl OracleState {
    /// Stores one point. If both RTDS legacy and PolyBolt deliver the same (asset, kind, ts),
    /// the PolyBolt value wins regardless of arrival order.
    pub fn insert(
        &mut self,
        asset: Asset,
        kind: OracleKind,
        source: Source,
        ts_ms: i64,
        value: f64,
    ) {
        let points = self.series.entry((asset, slot(kind))).or_default();
        let new = Point { value, source };
        match points.get(&ts_ms) {
            Some(old) if old.source == Source::Polybolt && source != Source::Polybolt => {}
            _ => {
                points.insert(ts_ms, new);
            }
        }
    }

    /// The newest point: `(ts_ms, value, source)`.
    pub fn latest(&self, asset: Asset, kind: OracleKind) -> Option<(i64, f64, Source)> {
        let (ts, p) = self.series.get(&(asset, slot(kind)))?.last_key_value()?;
        Some((*ts, p.value, p.source))
    }

    /// The point at exactly `ts_ms`; no interpolation and no nearest match.
    pub fn value_at(&self, asset: Asset, kind: OracleKind, ts_ms: i64) -> Option<(f64, Source)> {
        let p = self.series.get(&(asset, slot(kind)))?.get(&ts_ms)?;
        Some((p.value, p.source))
    }

    /// Every point as `(unix s, value)`, ascending, the last point of a second winning.
    pub fn seconds(&self, asset: Asset, kind: OracleKind) -> Vec<(i64, f64)> {
        let mut out: Vec<(i64, f64)> = Vec::new();
        for (ts, p) in self.series.get(&(asset, slot(kind))).into_iter().flatten() {
            let s = ts.div_euclid(1000);
            match out.last_mut() {
                Some(last) if last.0 == s => last.1 = p.value,
                _ => out.push((s, p.value)),
            }
        }
        out
    }

    /// Drops points older than `now_ms - RETENTION_MS`.
    pub fn prune(&mut self, now_ms: i64) {
        let cutoff = now_ms - RETENTION_MS;
        for points in self.series.values_mut() {
            *points = points.split_off(&cutoff);
        }
    }
}

/// `OracleKind` is not `Ord`, so the map is keyed by a slot index instead.
fn slot(kind: OracleKind) -> usize {
    match kind {
        OracleKind::Chainlink => 0,
        OracleKind::Twap60 => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTC: Asset = Asset::Btc;
    const TWAP: OracleKind = OracleKind::Twap60;

    #[test]
    fn value_at_exact_ts_only() {
        let mut o = OracleState::default();
        o.insert(BTC, TWAP, Source::Polybolt, 1000, 1.0);
        o.insert(BTC, TWAP, Source::Polybolt, 2000, 2.0);
        assert_eq!(o.value_at(BTC, TWAP, 1500), None);
        assert_eq!(o.value_at(BTC, TWAP, 2000), Some((2.0, Source::Polybolt)));
        assert_eq!(o.value_at(BTC, OracleKind::Chainlink, 2000), None);
        assert_eq!(o.value_at(Asset::Eth, TWAP, 2000), None);
        assert_eq!(o.latest(BTC, TWAP), Some((2000, 2.0, Source::Polybolt)));
    }

    #[test]
    fn polybolt_preferred_over_rtds_same_ts() {
        let mut o = OracleState::default();
        o.insert(BTC, TWAP, Source::Polybolt, 1000, 1.0);
        o.insert(BTC, TWAP, Source::RtdsLegacy, 1000, 9.0);
        assert_eq!(o.value_at(BTC, TWAP, 1000), Some((1.0, Source::Polybolt)));

        o.insert(BTC, TWAP, Source::RtdsLegacy, 2000, 9.0);
        o.insert(BTC, TWAP, Source::Polybolt, 2000, 2.0);
        assert_eq!(o.value_at(BTC, TWAP, 2000), Some((2.0, Source::Polybolt)));
    }

    #[test]
    fn seconds_are_ascending_and_one_per_second() {
        let mut o = OracleState::default();
        o.insert(BTC, TWAP, Source::Polybolt, 2_000, 2.0);
        o.insert(BTC, TWAP, Source::Polybolt, 1_000, 1.0);
        o.insert(BTC, TWAP, Source::Polybolt, 1_500, 1.5);
        assert_eq!(o.seconds(BTC, TWAP), vec![(1, 1.5), (2, 2.0)]);
        assert!(o.seconds(Asset::Eth, TWAP).is_empty());
    }

    #[test]
    fn prune_keeps_last_15_minutes() {
        let mut o = OracleState::default();
        o.insert(BTC, TWAP, Source::Polybolt, 1_000, 1.0);
        o.insert(BTC, TWAP, Source::Polybolt, 2_000, 2.0);
        o.prune(2_000 + RETENTION_MS);
        assert_eq!(o.value_at(BTC, TWAP, 1_000), None);
        assert_eq!(o.value_at(BTC, TWAP, 2_000), Some((2.0, Source::Polybolt)));
    }
}
