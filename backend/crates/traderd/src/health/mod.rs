//! Health monitors: clock offset, machine sleep, geoblock, and the safety roll-up.

pub mod clock;
pub mod geoblock;
pub mod sleep;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use raki_core::{
    ClockStatus, FeedStatusDto, GeoStatus, RecorderStatus, SafetyChecks, StoreStatus, now_ms,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

/// The recorder must have flushed within this window for `recorder_ok`.
const MAX_FLUSH_AGE_MS: i64 = 5_000;

struct Inner {
    clock: ClockStatus,
    geo: GeoStatus,
    sleep_events: u32,
    /// A clock or geoblock result older than this is treated as unknown.
    max_ages: MaxAges,
}

/// How old a monitor result may be before the safety checks stop trusting it: twice the
/// monitor's interval. A result from before a sleep, or from a monitor that died, must not keep
/// passing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaxAges {
    pub clock_ms: i64,
    pub geo_ms: i64,
}

impl MaxAges {
    pub fn from_intervals(clock: Duration, geo: Duration) -> Self {
        MaxAges {
            clock_ms: 2 * clock.as_millis() as i64,
            geo_ms: 2 * geo.as_millis() as i64,
        }
    }
}

impl Default for MaxAges {
    /// For the default 60 s clock and 300 s geoblock intervals.
    fn default() -> Self {
        MaxAges::from_intervals(Duration::from_secs(60), Duration::from_secs(300))
    }
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
                max_ages: MaxAges::default(),
            }),
        })
    }

    pub fn set_max_ages(&self, max_ages: MaxAges) {
        self.lock().max_ages = max_ages;
    }

    pub fn max_ages(&self) -> MaxAges {
        self.lock().max_ages
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

/// Resolves when the machine woke from sleep (the sleep monitor's reconnect broadcast). A lagged
/// receiver still counts; a closed channel never fires.
pub async fn woke(rx: &mut broadcast::Receiver<()>) {
    match rx.recv().await {
        Ok(()) | Err(RecvError::Lagged(_)) => {}
        Err(RecvError::Closed) => std::future::pending().await,
    }
}

/// Everything the safety roll-up looks at.
#[derive(Clone, Copy)]
pub struct SafetyInputs<'a> {
    pub feeds: &'a [FeedStatusDto],
    pub clock: &'a ClockStatus,
    pub geo: &'a GeoStatus,
    pub rec: &'a RecorderStatus,
    pub rec_dropped_last_min: u64,
    pub store: &'a StoreStatus,
    pub max_ages: MaxAges,
}

/// Rolls the individual health facts up into the checks that gate trading.
pub fn safety(inputs: &SafetyInputs<'_>) -> SafetyChecks {
    safety_at(now_ms(), inputs)
}

/// Whether a check done at `checked_ms` is too old to trust.
fn too_old(checked_ms: Option<i64>, now_ms: i64, max_age_ms: i64) -> Option<i64> {
    let age = now_ms - checked_ms?;
    (age > max_age_ms).then_some(age)
}

fn safety_at(now_ms: i64, i: &SafetyInputs<'_>) -> SafetyChecks {
    let SafetyInputs {
        feeds,
        clock,
        geo,
        rec,
        rec_dropped_last_min,
        store,
        max_ages,
    } = *i;
    let mut reasons = Vec::new();

    let mut feeds_fresh = true;
    for f in feeds.iter().filter(|f| f.enabled && f.stale) {
        feeds_fresh = false;
        reasons.push(format!("feed stale: {}", f.source.as_str()));
    }
    for f in feeds
        .iter()
        .filter(|f| f.enabled && f.events_dropped_recent > 0)
    {
        feeds_fresh = false;
        reasons.push(format!(
            "engine dropped {} {} events in the last minute",
            f.events_dropped_recent,
            f.source.as_str()
        ));
    }

    let clock_stale = too_old(clock.checked_ms, now_ms, max_ages.clock_ms);
    let clock_ok = clock.ok && clock.offset_ms.is_some() && clock_stale.is_none();
    if !clock_ok {
        match (clock_stale, clock.offset_ms) {
            (Some(age), _) => reasons.push(format!("clock check stale: {} s old", age / 1000)),
            (None, None) => reasons.push("clock offset unknown".to_owned()),
            (None, Some(o)) => reasons.push(format!(
                "clock offset {:.1} ms > {} ms",
                o.abs(),
                clock.max_offset_ms
            )),
        }
    }

    let geo_stale = too_old(geo.checked_ms, now_ms, max_ages.geo_ms);
    let geoblock_ok = geo.blocked == Some(false) && geo_stale.is_none();
    match (geo.blocked, geo_stale) {
        (Some(true), _) => reasons.push(format!(
            "geoblocked: {}",
            geo.country.as_deref().unwrap_or("unknown country")
        )),
        (Some(false), Some(age)) => {
            reasons.push(format!("geoblock check stale: {} s old", age / 1000))
        }
        (Some(false), None) => {}
        (None, _) => reasons.push("geoblock status unknown".to_owned()),
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

    let store_ok = store.ok;
    if !store_ok {
        reasons.push(format!(
            "database writes failing ({} pending): {}",
            store.pending_writes,
            store.last_error.as_deref().unwrap_or("unknown error")
        ));
    }

    SafetyChecks {
        feeds_fresh,
        clock_ok,
        geoblock_ok,
        recorder_ok,
        store_ok,
        all_ok: feeds_fresh && clock_ok && geoblock_ok && recorder_ok && store_ok,
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
            events_dropped: 0,
            events_dropped_recent: 0,
        }
    }

    fn clock(offset: Option<f64>, ok: bool) -> ClockStatus {
        ClockStatus {
            offset_ms: offset,
            ok,
            checked_ms: Some(NOW - 1_000),
            max_offset_ms: 50.0,
        }
    }

    fn geo(blocked: Option<bool>) -> GeoStatus {
        GeoStatus {
            blocked,
            country: Some("US".into()),
            checked_ms: Some(NOW - 1_000),
        }
    }

    fn store_ok() -> StoreStatus {
        StoreStatus {
            ok: true,
            pending_writes: 0,
            dropped_writes: 0,
            last_ok_ms: Some(NOW),
            last_error: None,
        }
    }

    /// `safety_at` with a healthy store and the default max ages.
    fn check(
        feeds: &[FeedStatusDto],
        clock: &ClockStatus,
        geo: &GeoStatus,
        rec: &RecorderStatus,
        dropped: u64,
    ) -> SafetyChecks {
        safety_at(
            NOW,
            &SafetyInputs {
                feeds,
                clock,
                geo,
                rec,
                rec_dropped_last_min: dropped,
                store: &store_ok(),
                max_ages: MaxAges::default(),
            },
        )
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

    const NOW: i64 = 10_000_000;

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
        let s = check(
            &feeds,
            &clock(Some(-3.0), true),
            &geo(Some(false)),
            &rec(Some(NOW - 500)),
            0,
        );
        assert!(s.all_ok, "{:?}", s.reasons);
        assert!(s.feeds_fresh && s.clock_ok && s.geoblock_ok && s.recorder_ok && s.store_ok);
        assert!(s.reasons.is_empty());
    }

    #[test]
    fn safety_reasons() {
        let feeds = [
            feed(Source::Binance, true, false),
            feed(Source::Coinbase, true, true),
        ];
        let s = check(
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
        let s = check(
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
            check(
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
    fn stale_clock_and_geoblock_results_are_not_trusted() {
        let ages = MaxAges {
            clock_ms: 120_000,
            geo_ms: 600_000,
        };
        let run = |clock_age: i64, geo_age: i64| {
            let mut c = clock(Some(1.0), true);
            c.checked_ms = Some(NOW - clock_age);
            let mut g = geo(Some(false));
            g.checked_ms = Some(NOW - geo_age);
            safety_at(
                NOW,
                &SafetyInputs {
                    feeds: &[],
                    clock: &c,
                    geo: &g,
                    rec: &rec(Some(NOW)),
                    rec_dropped_last_min: 0,
                    store: &store_ok(),
                    max_ages: ages,
                },
            )
        };
        let fresh = run(120_000, 600_000);
        assert!(fresh.clock_ok && fresh.geoblock_ok, "{:?}", fresh.reasons);
        // A pre-sleep "ok" an hour old passes nothing.
        let s = run(3_600_000, 3_600_000);
        assert!(!s.clock_ok && !s.geoblock_ok && !s.all_ok);
        assert!(
            s.reasons
                .contains(&"clock check stale: 3600 s old".to_owned())
        );
        assert!(
            s.reasons
                .contains(&"geoblock check stale: 3600 s old".to_owned())
        );
    }

    #[test]
    fn dropped_events_and_failing_store_fail_the_checks() {
        let mut f = feed(Source::ClobMarket, true, false);
        f.events_dropped_recent = 7;
        let failing = StoreStatus {
            ok: false,
            pending_writes: 3,
            dropped_writes: 0,
            last_ok_ms: None,
            last_error: Some("connection refused".into()),
        };
        let s = safety_at(
            NOW,
            &SafetyInputs {
                feeds: &[f],
                clock: &clock(Some(0.0), true),
                geo: &geo(Some(false)),
                rec: &rec(Some(NOW)),
                rec_dropped_last_min: 0,
                store: &failing,
                max_ages: MaxAges::default(),
            },
        );
        assert!(!s.feeds_fresh && !s.store_ok && !s.all_ok);
        assert!(s.clock_ok && s.geoblock_ok && s.recorder_ok);
        assert!(
            s.reasons
                .contains(&"engine dropped 7 clob_market events in the last minute".to_owned())
        );
        assert!(
            s.reasons
                .contains(&"database writes failing (3 pending): connection refused".to_owned())
        );
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
