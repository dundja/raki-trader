//! Detects that the machine slept: the monotonic clock keeps running through sleep, `Instant` does not.

use std::sync::Arc;
use std::time::{Duration, Instant};

use raki_core::{Source, now_ns};
use raki_recorder::{Record, RecordKind, RecordTx};
use tokio::sync::broadcast;
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

/// Polls every second. On a detected sleep: counts it, records a `Sleep` record and tells every
/// feed to reconnect, since their sockets died while the machine was asleep.
pub async fn run_sleep_monitor(
    state: Arc<HealthState>,
    rec: RecordTx,
    reconnect: broadcast::Sender<()>,
    shutdown: CancellationToken,
) {
    let mut detector = SleepDetector::new();
    let mut tick = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => {}
        }
        let Some(slept) = detector.poll() else {
            continue;
        };
        tracing::warn!(
            slept_ms = slept.as_millis() as u64,
            "machine slept; forcing feed reconnect"
        );
        state.record_sleep();
        rec.send(Record {
            t: now_ns(),
            s: Source::System,
            c: 0,
            k: RecordKind::Sleep,
            d: serde_json::json!({ "slept_ms": slept.as_millis() as u64 }).to_string(),
        });
        // No receivers just means no feed is running yet.
        let _ = reconnect.send(());
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
    async fn monitor_stops_on_shutdown() {
        let state = HealthState::new(50.0);
        let (rec, _rx) = raki_recorder::record_channel(4);
        let (tx, _rx2) = broadcast::channel(1);
        let shutdown = CancellationToken::new();
        let h = tokio::spawn(run_sleep_monitor(state.clone(), rec, tx, shutdown.clone()));
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sleep_events(), 0);
    }
}
