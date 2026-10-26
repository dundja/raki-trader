//! Live connection health of every feed, shared between the runners and the health snapshot.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use raki_core::{FeedStatusDto, Source};

/// `msgs_per_sec` is averaged over this trailing window.
const RATE_WINDOW_SECS: i64 = 10;
/// Staleness threshold for a source missing from the `stale_ms` map.
const DEFAULT_STALE_MS: i64 = 10_000;

#[derive(Debug)]
struct FeedState {
    enabled: bool,
    stale_ms: i64,
    connected: bool,
    last_msg_ms: Option<i64>,
    msgs_total: u64,
    /// Messages per epoch second, oldest first, covering at most the rate window.
    per_sec: VecDeque<(i64, u32)>,
    reconnects: u32,
    last_error: Option<String>,
}

impl FeedState {
    fn new(enabled: bool, stale_ms: i64) -> Self {
        FeedState {
            enabled,
            stale_ms,
            connected: false,
            last_msg_ms: None,
            msgs_total: 0,
            per_sec: VecDeque::new(),
            reconnects: 0,
            last_error: None,
        }
    }

    fn prune(&mut self, now_sec: i64) {
        while self
            .per_sec
            .front()
            .is_some_and(|&(sec, _)| sec <= now_sec - RATE_WINDOW_SECS)
        {
            self.per_sec.pop_front();
        }
    }
}

/// Per-source feed health. Runners report into it; the health snapshot reads it.
#[derive(Debug)]
pub struct FeedStatusRegistry {
    inner: Mutex<Registry>,
}

#[derive(Debug)]
struct Registry {
    /// Snapshot order: the order sources were registered in.
    order: Vec<Source>,
    feeds: HashMap<Source, FeedState>,
    stale_ms: HashMap<Source, i64>,
}

impl Registry {
    /// A source that reports without having been registered is treated as enabled.
    fn feed(&mut self, s: Source) -> &mut FeedState {
        if !self.feeds.contains_key(&s) {
            self.order.push(s);
        }
        let stale_ms = self.stale_ms.get(&s).copied().unwrap_or(DEFAULT_STALE_MS);
        self.feeds
            .entry(s)
            .or_insert_with(|| FeedState::new(true, stale_ms))
    }
}

impl FeedStatusRegistry {
    /// `enabled` lists every source the snapshot reports, in order. A disabled source is never
    /// stale.
    pub fn new(enabled: &[(Source, bool)], stale_ms: HashMap<Source, i64>) -> Arc<Self> {
        let mut order = Vec::with_capacity(enabled.len());
        let mut feeds = HashMap::with_capacity(enabled.len());
        for &(s, on) in enabled {
            if !feeds.contains_key(&s) {
                order.push(s);
            }
            let ms = stale_ms.get(&s).copied().unwrap_or(DEFAULT_STALE_MS);
            feeds.insert(s, FeedState::new(on, ms));
        }
        Arc::new(FeedStatusRegistry {
            inner: Mutex::new(Registry {
                order,
                feeds,
                stale_ms,
            }),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Registry) -> R) -> R {
        f(&mut self.inner.lock().expect("feed status mutex poisoned"))
    }

    pub fn on_connected(&self, s: Source) {
        self.with(|r| r.feed(s).connected = true);
    }

    /// Counts a reconnect when an open connection drops. A failed connect attempt only updates
    /// `last_error`.
    pub fn on_disconnected(&self, s: Source, err: Option<String>) {
        self.with(|r| {
            let f = r.feed(s);
            if f.connected {
                f.reconnects += 1;
            }
            f.connected = false;
            if err.is_some() {
                f.last_error = err;
            }
        });
    }

    pub fn on_message(&self, s: Source, now_ms: i64) {
        self.with(|r| {
            let f = r.feed(s);
            f.last_msg_ms = Some(now_ms);
            f.msgs_total += 1;
            let sec = now_ms.div_euclid(1_000);
            match f.per_sec.back_mut() {
                Some((last, n)) if *last == sec => *n += 1,
                _ => f.per_sec.push_back((sec, 1)),
            }
            f.prune(sec);
        });
    }

    /// `msgs_per_sec` covers the trailing 10 s. A feed is stale when it is enabled and either
    /// disconnected or silent for longer than its `stale_ms`.
    pub fn snapshot(&self, now_ms: i64) -> Vec<FeedStatusDto> {
        self.with(|r| {
            let Registry { order, feeds, .. } = r;
            order
                .iter()
                .map(|s| {
                    let f = feeds.get_mut(s).expect("ordered source is registered");
                    f.prune(now_ms.div_euclid(1_000));
                    let recent: u32 = f.per_sec.iter().map(|&(_, n)| n).sum();
                    let silent = f.last_msg_ms.is_none_or(|t| now_ms - t > f.stale_ms);
                    FeedStatusDto {
                        source: *s,
                        enabled: f.enabled,
                        connected: f.connected,
                        last_msg_ms: f.last_msg_ms,
                        msgs_total: f.msgs_total,
                        msgs_per_sec: f64::from(recent) / RATE_WINDOW_SECS as f64,
                        reconnects: f.reconnects,
                        last_error: f.last_error.clone(),
                        stale: f.enabled && (!f.connected || silent),
                    }
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_793_031_000_000;

    fn registry() -> Arc<FeedStatusRegistry> {
        FeedStatusRegistry::new(
            &[(Source::Binance, true), (Source::Polybolt, false)],
            HashMap::from([(Source::Binance, 5_000)]),
        )
    }

    fn get(r: &FeedStatusRegistry, s: Source, now: i64) -> FeedStatusDto {
        r.snapshot(now).into_iter().find(|f| f.source == s).unwrap()
    }

    #[test]
    fn snapshot_keeps_registration_order_and_disabled_is_never_stale() {
        let r = registry();
        let snap = r.snapshot(T0);
        assert_eq!(
            snap.iter().map(|f| f.source).collect::<Vec<_>>(),
            [Source::Binance, Source::Polybolt]
        );
        assert!(snap[0].stale, "enabled and never connected");
        assert!(!snap[1].enabled && !snap[1].stale);
    }

    #[test]
    fn fresh_then_stale_by_age() {
        let r = registry();
        r.on_connected(Source::Binance);
        r.on_message(Source::Binance, T0);
        assert!(!get(&r, Source::Binance, T0 + 5_000).stale);
        assert!(get(&r, Source::Binance, T0 + 5_001).stale);
    }

    #[test]
    fn rate_over_trailing_ten_seconds() {
        let r = registry();
        r.on_connected(Source::Binance);
        for i in 0..50 {
            r.on_message(Source::Binance, T0 + i * 100); // 5 s of 10 msg/s
        }
        let f = get(&r, Source::Binance, T0 + 5_000);
        assert_eq!(f.msgs_total, 50);
        assert!((f.msgs_per_sec - 5.0).abs() < 1e-9, "{}", f.msgs_per_sec);
        assert_eq!(get(&r, Source::Binance, T0 + 20_000).msgs_per_sec, 0.0);
    }

    #[test]
    fn reconnects_count_dropped_connections_only() {
        let r = registry();
        r.on_disconnected(Source::Binance, Some("refused".into()));
        assert_eq!(get(&r, Source::Binance, T0).reconnects, 0);
        r.on_connected(Source::Binance);
        r.on_disconnected(Source::Binance, None);
        let f = get(&r, Source::Binance, T0);
        assert_eq!(f.reconnects, 1);
        assert_eq!(f.last_error.as_deref(), Some("refused"));
        assert!(!f.connected && f.stale);
    }
}
