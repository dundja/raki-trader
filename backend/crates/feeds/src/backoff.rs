//! Exponential reconnect backoff with jitter.

use std::time::Duration;

/// Doubling backoff, capped at `max`. Each delay is drawn uniformly from `[d/2, d]`, so feeds that
/// drop together do not reconnect in lockstep.
#[derive(Debug)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    current: Duration,
    rng: u64,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        // xorshift must not start at zero.
        let seed = raki_core::now_ns() as u64 | 1;
        Backoff {
            initial,
            max,
            current: initial.min(max),
            rng: seed,
        }
    }

    /// The next delay; the one after it doubles, up to the max.
    pub fn next_delay(&mut self) -> Duration {
        let d = self.current;
        self.current = (d * 2).min(self.max);
        self.jitter(d)
    }

    /// The max delay, exactly; later delays stay at the max until [`Backoff::reset`]. For errors
    /// that a quick retry cannot fix (bad credentials, a policy close).
    pub fn max_delay(&mut self) -> Duration {
        self.current = self.max;
        self.max
    }

    pub fn reset(&mut self) {
        self.current = self.initial.min(self.max);
    }

    fn jitter(&mut self, d: Duration) -> Duration {
        let half = d / 2;
        let span = (d - half).as_nanos() as u64;
        half + Duration::from_nanos(self.xorshift() % (span + 1))
    }

    fn xorshift(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn doubles_with_jitter_up_to_max() {
        let mut b = Backoff::new(secs(1), secs(30));
        for cap in [1, 2, 4, 8, 16, 30, 30] {
            let d = b.next_delay();
            assert!(d >= secs(cap) / 2 && d <= secs(cap), "{d:?} vs cap {cap}s");
        }
    }

    #[test]
    fn reset_starts_over_and_max_delay_pins_max() {
        let mut b = Backoff::new(secs(1), secs(30));
        assert_eq!(b.max_delay(), secs(30));
        assert!(b.next_delay() >= secs(15));
        b.reset();
        assert!(b.next_delay() <= secs(1));
    }

    #[test]
    fn jitter_varies() {
        let mut b = Backoff::new(secs(30), secs(30));
        let delays: std::collections::HashSet<Duration> = (0..20).map(|_| b.next_delay()).collect();
        assert!(delays.len() > 1);
    }
}
