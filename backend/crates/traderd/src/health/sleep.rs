//! Detects that the machine slept: the monotonic clock keeps running through sleep, `Instant` does not.

use std::sync::Arc;
use std::time::{Duration, Instant};

use raki_core::{Source, now_ms, now_ns};
use raki_recorder::{Record, RecordKind, RecordTx};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::HealthState;

const POLL: Duration = Duration::from_secs(1);
/// A gap between the two clocks larger than this counts as a sleep.
const THRESHOLD: Duration = Duration::from_secs(2);

/// A clock that keeps counting while the machine sleeps. macOS `CLOCK_MONOTONIC` does; on Linux
/// that is `CLOCK_BOOTTIME` (Rust's `Instant` already uses `CLOCK_MONOTONIC` there).
#[cfg(target_os = "macos")]
const SLEEP_AWARE_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;
#[cfg(not(target_os = "macos"))]
const SLEEP_AWARE_CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;

fn monotonic_now() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call.
    let rc = unsafe { libc::clock_gettime(SLEEP_AWARE_CLOCK, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime failed");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub struct SleepDetector {
    last_instant: Instant,
    last_monotonic: Duration,
}

impl Default for SleepDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl SleepDetector {
    pub fn new() -> Self {
        SleepDetector {
            last_instant: Instant::now(),
            last_monotonic: monotonic_now(),
        }
    }

    /// Compares the time that passed on both clocks since the last reading. Returns how long the
    /// machine slept when the sleep-aware clock ran more than 2 s ahead of `Instant`.
    pub fn check(
        &mut self,
        instant_elapsed: Duration,
        monotonic_elapsed: Duration,
    ) -> Option<Duration> {
        let slept = monotonic_elapsed.checked_sub(instant_elapsed)?;
        (slept > THRESHOLD).then_some(slept)
    }

    /// Reads both clocks, runs [`check`](Self::check) on the time since the previous poll.
    pub fn poll(&mut self) -> Option<Duration> {
        let (instant, monotonic) = (Instant::now(), monotonic_now());
        let slept = self.check(
            instant.saturating_duration_since(self.last_instant),
            monotonic.saturating_sub(self.last_monotonic),
        );
        self.last_instant = instant;
        self.last_monotonic = monotonic;
        slept
    }
}

/// A detected sleep, for the session's gap log in Postgres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SleepNotice {
    pub wake_ms: i64,
    pub slept_ms: i64,
}

/// Where a detected sleep is reported.
pub struct SleepSinks {
    pub state: Arc<HealthState>,
    pub rec: RecordTx,
    /// Feeds reconnect, the clock and geoblock monitors recheck.
    pub wake: broadcast::Sender<()>,
    /// The engine loop stores the sleep as a gap and subtracts it from recorded time.
    pub notices: mpsc::Sender<SleepNotice>,
}

impl SleepSinks {
    /// Counts the sleep, records a raw `Sleep` record, sends the notice to the engine loop and
    /// broadcasts the wake, since every socket died while the machine was asleep.
    fn on_sleep(&self, slept: Duration, wake_ms: i64) {
        let slept_ms = slept.as_millis() as i64;
        tracing::warn!(
            slept_ms,
            "machine slept; forcing feed reconnect and health rechecks"
        );
        self.state.record_sleep();
        self.rec.send(Record {
            t: now_ns(),
            s: Source::System,
            c: 0,
            k: RecordKind::Sleep,
            d: serde_json::json!({ "slept_ms": slept_ms }).to_string(),
        });
        if self
            .notices
            .try_send(SleepNotice { wake_ms, slept_ms })
            .is_err()
        {
            tracing::error!(
                slept_ms,
                "sleep notice not delivered; it stays in the raw file only"
            );
        }
        // No receivers just means no feed is running yet.
        let _ = self.wake.send(());
    }
}

/// Polls every second and reports every detected sleep ([`SleepSinks::on_sleep`]).
pub async fn run_sleep_monitor(sinks: SleepSinks, shutdown: CancellationToken) {
    let mut detector = SleepDetector::new();
    let mut tick = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => {}
        }
        if let Some(slept) = detector.poll() {
            sinks.on_sleep(slept, now_ms());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_detector_flags_drift() {
        let mut d = SleepDetector::new();
        assert_eq!(
            d.check(Duration::from_secs(1), Duration::from_secs(1)),
            None
        );
        assert_eq!(
            d.check(Duration::from_secs(1), Duration::from_secs(5)),
            Some(Duration::from_secs(4))
        );
    }

    #[test]
    fn sleep_detector_ignores_small_drift_and_negative() {
        let mut d = SleepDetector::new();
        assert_eq!(
            d.check(Duration::from_secs(1), Duration::from_secs(3)),
            None
        );
        assert_eq!(
            d.check(Duration::from_secs(3), Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn poll_without_sleep_is_quiet() {
        let mut d = SleepDetector::new();
        assert_eq!(d.poll(), None);
    }

    #[tokio::test]
    async fn a_sleep_is_counted_recorded_noticed_and_wakes_everyone() {
        let state = HealthState::new(50.0);
        let (rec, mut rec_rx) = raki_recorder::record_channel(4);
        let (wake, mut wake_rx) = broadcast::channel(1);
        let (notices, mut notice_rx) = mpsc::channel(4);
        let sinks = SleepSinks {
            state: state.clone(),
            rec,
            wake,
            notices,
        };
        sinks.on_sleep(Duration::from_secs(1800), 5_000_000);
        assert_eq!(state.sleep_events(), 1);
        let r = rec_rx.try_recv().unwrap();
        assert_eq!((r.s, r.k), (Source::System, RecordKind::Sleep));
        assert_eq!(r.d, r#"{"slept_ms":1800000}"#);
        assert_eq!(
            notice_rx.try_recv().unwrap(),
            SleepNotice {
                wake_ms: 5_000_000,
                slept_ms: 1_800_000
            }
        );
        assert!(wake_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn monitor_stops_on_shutdown() {
        let state = HealthState::new(50.0);
        let (rec, _rx) = raki_recorder::record_channel(4);
        let (tx, _rx2) = broadcast::channel(1);
        let (notices, _nrx) = mpsc::channel(1);
        let shutdown = CancellationToken::new();
        let sinks = SleepSinks {
            state: state.clone(),
            rec,
            wake: tx,
            notices,
        };
        let h = tokio::spawn(run_sleep_monitor(sinks, shutdown.clone()));
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sleep_events(), 0);
    }
}
