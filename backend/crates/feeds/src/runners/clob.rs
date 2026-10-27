//! Polymarket CLOB market channel (`wss://ws-subscriptions-clob.polymarket.com/ws/market`).

use std::collections::BTreeSet;
use std::time::Duration;

use raki_core::{Asset, Source};
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::Instant;

use super::{
    End, FeedCtx, Incoming, Link, Retry, Session, default_backoff, heartbeat, run_session,
};
use crate::backoff::Backoff;
use crate::parse::clob::parse_clob;

#[derive(Clone, Debug)]
pub struct ClobTiming {
    /// Text `PING` interval.
    pub ping: Duration,
    /// Reconnect when no `PONG` arrives for this long.
    pub pong_timeout: Duration,
    /// Reconnect when no data frame (book, price change, trade, ...) arrives for this long while
    /// subscribed. `PONG`s keep the socket alive but are not data.
    pub data_timeout: Duration,
    /// Reconnect delay after a server close that a quick reconnect fixes: 1000 (follows
    /// `market_resolved`) and 1013 ("slow consumer: send buffer full").
    pub clean_close_delay: Duration,
}

impl Default for ClobTiming {
    fn default() -> Self {
        ClobTiming {
            ping: Duration::from_secs(5),
            pong_timeout: Duration::from_secs(15),
            data_timeout: Duration::from_secs(30),
            clean_close_delay: Duration::from_millis(250),
        }
    }
}

/// Subscribes to every token in `desired` and follows its changes. With an empty set it stays
/// connected without subscribing and without pinging: the live server closes with 1008 on a
/// `PING` sent before any subscribe.
///
/// `shard` is the asset this connection covers when the CLOB is split into one connection per
/// asset: one socket for every token overflows the server's per-connection send buffer (close
/// 1013) at peak rates.
pub async fn run_clob(
    ctx: FeedCtx,
    url: String,
    shard: Option<Asset>,
    desired: watch::Receiver<BTreeSet<String>>,
) {
    let timing = ClobTiming::default();
    run_clob_with(ctx, url, shard, desired, default_backoff(), timing).await;
}

pub async fn run_clob_with(
    ctx: FeedCtx,
    url: String,
    shard: Option<Asset>,
    desired: watch::Receiver<BTreeSet<String>>,
    backoff: Backoff,
    timing: ClobTiming,
) {
    let clob = Clob {
        url,
        shard,
        desired,
        desired_open: true,
        timing,
    };
    run_session(ctx, clob, backoff).await;
}

struct Clob {
    url: String,
    shard: Option<Asset>,
    desired: watch::Receiver<BTreeSet<String>>,
    /// False once the sender is gone: the set can no longer change.
    desired_open: bool,
    timing: ClobTiming,
}

/// The first subscribe on a connection; only this form enables the custom-feature events
/// (`best_bid_ask`, `market_resolved`, ...).
#[derive(Serialize)]
struct InitialSubscribe<'a> {
    assets_ids: &'a BTreeSet<String>,
    #[serde(rename = "type")]
    kind: &'static str,
    custom_feature_enabled: bool,
}

#[derive(Serialize)]
struct Update<'a> {
    assets_ids: Vec<&'a String>,
    operation: &'static str,
}

/// What this connection is subscribed to.
#[derive(Default)]
struct Subscribed {
    /// The initial subscribe was sent.
    initial_sent: bool,
    ids: BTreeSet<String>,
}

impl Subscribed {
    /// Brings the connection's subscription to `want`.
    async fn sync(&mut self, link: &mut Link<'_>, want: BTreeSet<String>) -> Result<(), End> {
        if !self.initial_sent {
            if want.is_empty() {
                return Ok(());
            }
            let frame = InitialSubscribe {
                assets_ids: &want,
                kind: "market",
                custom_feature_enabled: true,
            };
            link.send(to_json(&frame)).await?;
            self.initial_sent = true;
        } else {
            let added: Vec<&String> = want.difference(&self.ids).collect();
            let removed: Vec<&String> = self.ids.difference(&want).collect();
            for (ids, operation) in [(added, "subscribe"), (removed, "unsubscribe")] {
                if !ids.is_empty() {
                    let frame = Update {
                        assets_ids: ids,
                        operation,
                    };
                    link.send(to_json(&frame)).await?;
                }
            }
        }
        self.ids = want;
        Ok(())
    }
}

fn to_json(v: &impl Serialize) -> String {
    serde_json::to_string(v).expect("frame serializes")
}

impl Session for Clob {
    const SOURCE: Source = Source::ClobMarket;

    fn url(&self) -> &str {
        &self.url
    }

    /// Unsubscribed, nothing is expected; the data deadline starts with the first subscribe.
    fn idle_timeout(&self) -> Option<Duration> {
        None
    }

    fn shard(&self) -> Option<Asset> {
        self.shard
    }

    /// The heartbeat starts with the first subscribe: the live server closes with 1008
    /// ("invalid subscription payload") when a `PING` comes before any subscribe.
    async fn run(&mut self, link: &mut Link<'_>) -> End {
        let mut subscribed = Subscribed::default();
        let want = self.desired.borrow_and_update().clone();
        if let Err(end) = subscribed.sync(link, want).await {
            return end;
        }
        if subscribed.initial_sent {
            link.set_idle_timeout(Some(self.timing.data_timeout));
        }
        let mut ping = heartbeat(self.timing.ping);
        let mut pong_deadline = Instant::now() + self.timing.pong_timeout;
        loop {
            let beating = subscribed.initial_sent;
            tokio::select! {
                incoming = link.recv() => match incoming {
                    Incoming::Text { text, recv_ms } => {
                        if text.as_str().trim() == "PONG" {
                            pong_deadline = Instant::now() + self.timing.pong_timeout;
                        }
                        link.forward(parse_clob(text.as_str(), recv_ms));
                    }
                    Incoming::End(end) => return self.on_end(end, link.texts()),
                },
                _ = ping.tick(), if beating => {
                    if let Err(end) = link.send("PING".to_owned()).await {
                        return end;
                    }
                }
                () = tokio::time::sleep_until(pong_deadline), if beating => {
                    return End::error(format!("no PONG for {:?}", self.timing.pong_timeout));
                }
                changed = self.desired.changed(), if self.desired_open => match changed {
                    Ok(()) => {
                        let want = self.desired.borrow_and_update().clone();
                        if let Err(end) = subscribed.sync(link, want).await {
                            return end;
                        }
                        if !beating && subscribed.initial_sent {
                            ping = heartbeat(self.timing.ping);
                            pong_deadline = Instant::now() + self.timing.pong_timeout;
                            link.set_idle_timeout(Some(self.timing.data_timeout));
                        }
                    }
                    Err(_) => self.desired_open = false,
                },
            }
        }
    }
}

impl Clob {
    /// The server closes with 1000 right after `market_resolved`, and with 1013 ("slow consumer:
    /// send buffer full") when a burst overflows its per-connection buffer. Both are fixed by
    /// reconnecting at once: do so without growing the backoff, so the gap stays near 250 ms
    /// even when closes come less than a minute apart. A connection closed before any frame
    /// arrived takes the normal backoff, so a server that rejects us at once is not hammered
    /// every 250 ms.
    fn on_end(&self, end: End, texts: u64) -> End {
        if matches!(end.close_code(), Some(1000 | 1013)) && texts > 0 {
            end.retry(Retry::After(self.timing.clean_close_delay))
        } else {
            end
        }
    }
}
