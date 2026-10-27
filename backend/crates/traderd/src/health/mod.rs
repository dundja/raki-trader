//! Health monitors: clock offset, machine sleep, geoblock, and the safety roll-up.

pub mod clock;
pub mod geoblock;
pub mod sleep;

use std::sync::{Arc, Mutex, MutexGuard};

use raki_core::{ClockStatus, FeedStatusDto, GeoStatus, RecorderStatus, SafetyChecks, now_ms};

/// The recorder must have flushed within this window for `recorder_ok`.
const MAX_FLUSH_AGE_MS: i64 = 5_000;

struct Inner {
    clock: ClockStatus,
    geo: GeoStatus,
    sleep_events: u32,
}

/// Latest results of the monitors. Written by the monitor tasks, read by the health snapshot.
pub struct HealthState {
    inner: Mutex<Inner>,
}

impl HealthState {
    pub fn new(max_offset_ms: f64) -> Arc<Self> {
        Arc::new(HealthState {
            inner: Mutex::new(Inner {
                clock: ClockStatus {
                    offset_ms: None,
                    ok: false,
                    checked_ms: None,
                    max_offset_ms,
                },
                geo: GeoStatus {
                    blocked: None,
                    country: None,
                    checked_ms: None,
                },
                sleep_events: 0,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn clock(&self) -> ClockStatus {
        self.lock().clock.clone()
    }

    pub fn geo(&self) -> GeoStatus {
        self.lock().geo.clone()
    }

    pub fn sleep_events(&self) -> u32 {
        self.lock().sleep_events
    }

    /// Stores a clock check. `None` means no server answered: the offset is unknown, hence not ok.
    pub(crate) fn set_clock_offset(&self, offset_ms: Option<f64>) {
        let mut g = self.lock();
        g.clock.ok = offset_ms.is_some_and(|o| o.abs() <= g.clock.max_offset_ms);
        g.clock.offset_ms = offset_ms;
        g.clock.checked_ms = Some(now_ms());
    }

    pub(crate) fn set_geo(&self, blocked: Option<bool>, country: Option<String>) {
        let mut g = self.lock();
        g.geo = GeoStatus {
            blocked,
            country,
            checked_ms: Some(now_ms()),
        };
    }

    pub(crate) fn record_sleep(&self) {
        self.lock().sleep_events += 1;
    }
}

/// Median of the samples (mean of the middle two for an even count). `None` when empty.
pub fn median_offset_ms(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// Rolls the individual health facts up into the checks that gate trading.
pub fn safety(
    feeds: &[FeedStatusDto],
    clock: &ClockStatus,
    geo: &GeoStatus,
    rec: &RecorderStatus,
    rec_dropped_last_min: u64,
) -> SafetyChecks {
    safety_at(now_ms(), feeds, clock, geo, rec, rec_dropped_last_min)
}

fn safety_at(
    now_ms: i64,
    feeds: &[FeedStatusDto],
    clock: &ClockStatus,
    geo: &GeoStatus,
    rec: &RecorderStatus,
    rec_dropped_last_min: u64,
) -> SafetyChecks {
    let mut reasons = Vec::new();

    let mut feeds_fresh = true;
    for f in feeds.iter().filter(|f| f.enabled && f.stale) {
        feeds_fresh = false;
        reasons.push(format!("feed stale: {}", f.source.as_str()));
    }

    let clock_ok = clock.ok && clock.offset_ms.is_some();
    if !clock_ok {
        match clock.offset_ms {
            None => reasons.push("clock offset unknown".to_owned()),
            Some(o) => reasons.push(format!(
                "clock offset {:.1} ms > {} ms",
                o.abs(),
                clock.max_offset_ms
            )),
        }
    }

    let geoblock_ok = geo.blocked == Some(false);
    match geo.blocked {
        Some(false) => {}
        Some(true) => reasons.push(format!(
            "geoblocked: {}",
            geo.country.as_deref().unwrap_or("unknown country")
        )),
        None => reasons.push("geoblock status unknown".to_owned()),
    }

    let mut recorder_ok = true;
    if rec_dropped_last_min > 0 {
        recorder_ok = false;
        reasons.push(format!(
            "recorder dropped {rec_dropped_last_min} records in the last minute"
        ));
    }
    match rec.last_flush_ms {
        None => {
            recorder_ok = false;
            reasons.push("recorder has not flushed yet".to_owned());
        }
        Some(t) if now_ms - t > MAX_FLUSH_AGE_MS => {
            recorder_ok = false;
            reasons.push(format!("recorder flush stale: {} ms ago", now_ms - t));
        }
        Some(_) => {}
    }

    SafetyChecks {
        feeds_fresh,
        clock_ok,
        geoblock_ok,
        recorder_ok,
        all_ok: feeds_fresh && clock_ok && geoblock_ok && recorder_ok,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use raki_core::Source;

    use super::*;

    fn feed(source: Source, enabled: bool, stale: bool) -> FeedStatusDto {
        FeedStatusDto {
            source,
            enabled,
            connected: !stale,
            last_msg_ms: Some(1),
            msgs_total: 10,
            msgs_per_sec: 1.0,
            reconnects: 0,
            last_error: None,
            stale,
        }
    }

    fn clock(offset: Option<f64>, ok: bool) -> ClockStatus {
        ClockStatus {
            offset_ms: offset,
            ok,
            checked_ms: Some(1),
            max_offset_ms: 50.0,
        }
    }

    fn geo(blocked: Option<bool>) -> GeoStatus {
        GeoStatus {
            blocked,
            country: Some("US".into()),
            checked_ms: Some(1),
        }
    }

    fn rec(last_flush_ms: Option<i64>) -> RecorderStatus {
        RecorderStatus {
            file: None,
            records: 1,
            bytes: 1,
            dropped: 0,
            last_flush_ms,
        }
    }

    const NOW: i64 = 1_000_000;

    #[test]
    fn median_offset() {
        assert_eq!(median_offset_ms(&[1.0, 50.0, 3.0]), Some(3.0));
        assert_eq!(median_offset_ms(&[1.0, 3.0]), Some(2.0));
        assert_eq!(median_offset_ms(&[]), None);
    }

    #[test]
    fn safety_all_ok() {
        let feeds = [
            feed(Source::Binance, true, false),
            feed(Source::Coinbase, false, true),
        ];
        let s = safety_at(
            NOW,
            &feeds,
            &clock(Some(-3.0), true),
            &geo(Some(false)),
            &rec(Some(NOW - 500)),
            0,
        );
        assert!(s.all_ok, "{:?}", s.reasons);
        assert!(s.feeds_fresh && s.clock_ok && s.geoblock_ok && s.recorder_ok);
        assert!(s.reasons.is_empty());
    }

    #[test]
    fn safety_reasons() {
        let feeds = [
            feed(Source::Binance, true, false),
            feed(Source::Coinbase, true, true),
        ];
        let s = safety_at(
            NOW,
            &feeds,
            &clock(Some(63.1), false),
            &geo(None),
            &rec(Some(NOW)),
            0,
        );
        assert!(!s.all_ok);
        assert!(!s.feeds_fresh && !s.clock_ok && !s.geoblock_ok && s.recorder_ok);
        for want in [
            "feed stale: coinbase",
            "clock offset 63.1 ms > 50 ms",
            "geoblock status unknown",
        ] {
            assert!(
                s.reasons.iter().any(|r| r == want),
                "{want} not in {:?}",
                s.reasons
            );
        }
    }

    #[test]
    fn safety_unknown_clock_and_blocked_geo() {
        let s = safety_at(
            NOW,
            &[],
            &clock(None, false),
            &geo(Some(true)),
            &rec(Some(NOW)),
            0,
        );
        assert!(s.reasons.contains(&"clock offset unknown".to_owned()));
        assert!(s.reasons.contains(&"geoblocked: US".to_owned()));
        assert!(!s.clock_ok && !s.geoblock_ok);
    }

    #[test]
    fn safety_recorder_checks() {
        let ok = |dropped, flush| {
            safety_at(
                NOW,
                &[],
                &clock(Some(0.0), true),
                &geo(Some(false)),
                &rec(flush),
                dropped,
            )
            .recorder_ok
        };
        assert!(ok(0, Some(NOW - 5_000)));
        assert!(!ok(1, Some(NOW)));
        assert!(!ok(0, Some(NOW - 5_001)));
        assert!(!ok(0, None));
    }

    #[test]
    fn state_tracks_clock_geo_and_sleep() {
        let st = HealthState::new(50.0);
        assert!(!st.clock().ok && st.geo().blocked.is_none() && st.sleep_events() == 0);
        st.set_clock_offset(Some(-49.0));
        assert!(st.clock().ok);
        st.set_clock_offset(Some(63.1));
        assert!(!st.clock().ok);
        st.set_clock_offset(None);
        assert!(!st.clock().ok && st.clock().offset_ms.is_none());
        st.set_geo(Some(true), Some("US".into()));
        assert_eq!(st.geo().blocked, Some(true));
        st.record_sleep();
        assert_eq!(st.sleep_events(), 1);
    }
}
