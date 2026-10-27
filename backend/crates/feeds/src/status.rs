//! Live connection health of every feed, shared between the runners and the health snapshot.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use raki_core::{FeedStatusDto, Source};

/// `msgs_per_sec` is averaged over this trailing window.
const RATE_WINDOW_SECS: i64 = 10;
/// `events_dropped_recent` covers this trailing window.
const DROP_WINDOW_SECS: i64 = 60;
/// Staleness threshold for a source missing from the `stale_ms` map.
const DEFAULT_STALE_MS: i64 = 10_000;

#[derive(Debug)]
struct FeedState {
    enabled: bool,
    stale_ms: i64,
    /// Connections currently open. A sharded feed (CLOB) runs several.
    open: u32,
    /// Connections the feed runs when healthy; `connected` needs all of them open.
    want: u32,
    /// Last data message (control frames such as pongs and heartbeats do not count).
    last_msg_ms: Option<i64>,
    msgs_total: u64,
    /// Data messages per epoch second, oldest first, covering at most the rate window.
    per_sec: VecDeque<(i64, u32)>,
    reconnects: u32,
    last_error: Option<String>,
    /// Parsed events the engine never got because its channel was full.
    dropped_total: u64,
    /// Dropped events per epoch second, covering at most the drop window.
    dropped_per_sec: VecDeque<(i64, u32)>,
}

impl FeedState {
    fn new(enabled: bool, stale_ms: i64) -> Self {
        FeedState {
            enabled,
            stale_ms,
            open: 0,
            want: 1,
            last_msg_ms: None,
            msgs_total: 0,
            per_sec: VecDeque::new(),
            reconnects: 0,
            last_error: None,
            dropped_total: 0,
            dropped_per_sec: VecDeque::new(),
        }
    }

    fn prune(&mut self, now_sec: i64) {
        prune(&mut self.per_sec, now_sec - RATE_WINDOW_SECS);
        prune(&mut self.dropped_per_sec, now_sec - DROP_WINDOW_SECS);
    }
}

/// Drops the buckets at or before `oldest_excluded`.
fn prune(buckets: &mut VecDeque<(i64, u32)>, oldest_excluded: i64) {
    while buckets
        .front()
        .is_some_and(|&(sec, _)| sec <= oldest_excluded)
    {
        buckets.pop_front();
    }
}

fn bump(buckets: &mut VecDeque<(i64, u32)>, sec: i64) {
    match buckets.back_mut() {
        Some((last, n)) if *last == sec => *n += 1,
        _ => buckets.push_back((sec, 1)),
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

    /// How many connections `s` runs when healthy (default 1). The feed counts as connected only
    /// while all of them are open.
    pub fn expect_connections(&self, s: Source, n: u32) {
        self.with(|r| r.feed(s).want = n.max(1));
    }

    pub fn on_connected(&self, s: Source) {
        self.with(|r| r.feed(s).open += 1);
    }

    /// An open connection dropped: counts a reconnect.
    pub fn on_disconnected(&self, s: Source, err: Option<String>) {
        self.with(|r| {
            let f = r.feed(s);
            f.open = f.open.saturating_sub(1);
            f.reconnects += 1;
            if err.is_some() {
                f.last_error = err;
            }
        });
    }

    /// An open connection closed on shutdown: not a reconnect.
    pub fn on_stopped(&self, s: Source) {
        self.with(|r| {
            let f = r.feed(s);
            f.open = f.open.saturating_sub(1);
        });
    }

    /// A connect attempt failed; no connection was open.
    pub fn on_connect_failed(&self, s: Source, err: String) {
        self.with(|r| r.feed(s).last_error = Some(err));
    }

    /// A data message arrived. Control frames (pongs, heartbeats, acks) must not be reported here,
    /// or a feed whose subscriptions stopped delivering would still look fresh.
    pub fn on_message(&self, s: Source, now_ms: i64) {
        self.with(|r| {
            let f = r.feed(s);
            f.last_msg_ms = Some(now_ms);
            f.msgs_total += 1;
            let sec = now_ms.div_euclid(1_000);
            bump(&mut f.per_sec, sec);
            f.prune(sec);
        });
    }

    /// A parsed event was dropped because the engine's channel was full.
    pub fn on_event_dropped(&self, s: Source, now_ms: i64) {
        self.with(|r| {
            let f = r.feed(s);
            f.dropped_total += 1;
            let sec = now_ms.div_euclid(1_000);
            bump(&mut f.dropped_per_sec, sec);
            f.prune(sec);
        });
    }

    /// `msgs_per_sec` covers the trailing 10 s, `events_dropped_recent` the trailing 60 s. A feed
    /// is stale when it is enabled and either not fully connected or without data for longer
    /// than its `stale_ms`.
    pub fn snapshot(&self, now_ms: i64) -> Vec<FeedStatusDto> {
        self.with(|r| {
            let Registry { order, feeds, .. } = r;
            order
                .iter()
                .map(|s| {
                    let f = feeds.get_mut(s).expect("ordered source is registered");
                    f.prune(now_ms.div_euclid(1_000));
                    let recent: u32 = f.per_sec.iter().map(|&(_, n)| n).sum();
                    let dropped_recent: u32 = f.dropped_per_sec.iter().map(|&(_, n)| n).sum();
                    let silent = f.last_msg_ms.is_none_or(|t| now_ms - t > f.stale_ms);
                    let connected = f.open >= f.want;
                    FeedStatusDto {
                        source: *s,
                        enabled: f.enabled,
                        connected,
                        last_msg_ms: f.last_msg_ms,
                        msgs_total: f.msgs_total,
                        msgs_per_sec: f64::from(recent) / RATE_WINDOW_SECS as f64,
                        reconnects: f.reconnects,
                        last_error: f.last_error.clone(),
                        stale: f.enabled && (!connected || silent),
                        events_dropped: f.dropped_total,
                        events_dropped_recent: dropped_recent,
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
        r.on_connect_failed(Source::Binance, "refused".into());
        assert_eq!(get(&r, Source::Binance, T0).reconnects, 0);
        r.on_connected(Source::Binance);
        r.on_disconnected(Source::Binance, None);
        let f = get(&r, Source::Binance, T0);
        assert_eq!(f.reconnects, 1);
        assert_eq!(f.last_error.as_deref(), Some("refused"));
        assert!(!f.connected && f.stale);
        // A shutdown close is not a reconnect.
        r.on_connected(Source::Binance);
        r.on_stopped(Source::Binance);
        let f = get(&r, Source::Binance, T0);
        assert_eq!(f.reconnects, 1);
        assert!(!f.connected);
    }

    #[test]
    fn sharded_feed_is_connected_only_with_every_shard_open() {
        let r = registry();
        r.expect_connections(Source::Binance, 3);
        r.on_message(Source::Binance, T0);
        r.on_connected(Source::Binance);
        r.on_connected(Source::Binance);
        let f = get(&r, Source::Binance, T0);
        assert!(!f.connected && f.stale, "2 of 3 shards open");
        r.on_connected(Source::Binance);
        assert!(get(&r, Source::Binance, T0).connected);
        // One shard failing to reconnect keeps the feed down even when another shard flaps.
        r.on_disconnected(Source::Binance, Some("1013".into()));
        r.on_connect_failed(Source::Binance, "refused".into());
        r.on_disconnected(Source::Binance, None);
        r.on_connected(Source::Binance);
        let f = get(&r, Source::Binance, T0);
        assert!(!f.connected && f.stale);
        assert_eq!(f.reconnects, 2);
    }

    #[test]
    fn dropped_events_counted_over_the_last_minute() {
        let r = registry();
        r.on_event_dropped(Source::Binance, T0);
        r.on_event_dropped(Source::Binance, T0 + 500);
        let f = get(&r, Source::Binance, T0 + 1_000);
        assert_eq!((f.events_dropped, f.events_dropped_recent), (2, 2));
        let f = get(&r, Source::Binance, T0 + 60_000);
        assert_eq!((f.events_dropped, f.events_dropped_recent), (2, 0));
    }
}
