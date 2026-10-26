//! Legacy RTDS (`wss://ws-live-data.polymarket.com`): Chainlink price and 60 s TWAP, no auth.

use std::time::Duration;

use raki_core::{Asset, Source};
use serde::Serialize;

use super::{End, FeedCtx, Incoming, Link, Session, default_backoff, heartbeat, run_session};
use crate::backoff::Backoff;
use crate::parse::rtds::parse_rtds;

#[derive(Clone, Debug)]
pub struct RtdsTiming {
    /// Text `ping` interval. The server does not answer it.
    pub ping: Duration,
    /// Reconnect when nothing arrives for this long.
    pub stale_after: Duration,
}

impl Default for RtdsTiming {
    fn default() -> Self {
        RtdsTiming {
            ping: Duration::from_secs(5),
            stale_after: Duration::from_secs(15),
        }
    }
}

pub async fn run_rtds_legacy(ctx: FeedCtx, url: String, assets: Vec<Asset>) {
    run_rtds_legacy_with(ctx, url, assets, default_backoff(), RtdsTiming::default()).await;
}

pub async fn run_rtds_legacy_with(
    ctx: FeedCtx,
    url: String,
    assets: Vec<Asset>,
    backoff: Backoff,
    timing: RtdsTiming,
) {
    let subscribe = subscribe_frame(&assets);
    let rtds = Rtds {
        url,
        subscribe,
        timing,
    };
    run_session(ctx, rtds, backoff).await;
}

struct Rtds {
    url: String,
    subscribe: String,
    timing: RtdsTiming,
}

#[derive(Serialize)]
struct Subscribe {
    action: &'static str,
    subscriptions: Vec<Subscription>,
}

#[derive(Serialize)]
struct Subscription {
    topic: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    /// A JSON-encoded string, e.g. `{"symbol":"btc/usd"}`.
    filters: String,
}

/// The TWAP and Chainlink topics for every asset, in one frame.
fn subscribe_frame(assets: &[Asset]) -> String {
    let subscriptions = assets
        .iter()
        .flat_map(|a| {
            let filters = serde_json::json!({ "symbol": a.rtds_symbol() }).to_string();
            ["crypto_prices_twap_sixty", "crypto_prices_chainlink"].map(|topic| Subscription {
                topic,
                kind: "*",
                filters: filters.clone(),
            })
        })
        .collect();
    let frame = Subscribe {
        action: "subscribe",
        subscriptions,
    };
    serde_json::to_string(&frame).expect("frame serializes")
}

impl Session for Rtds {
    const SOURCE: Source = Source::RtdsLegacy;

    fn url(&self) -> &str {
        &self.url
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(self.timing.stale_after)
    }

    async fn run(&mut self, link: &mut Link<'_>) -> End {
        if let Err(end) = link.send(self.subscribe.clone()).await {
            return end;
        }
        let mut ping = heartbeat(self.timing.ping);
        loop {
            tokio::select! {
                incoming = link.recv() => match incoming {
                    Incoming::Text { text, recv_ms } => link.forward(parse_rtds(text.as_str(), recv_ms)),
                    Incoming::End(end) => return end,
                },
                _ = ping.tick() => {
                    if let Err(end) = link.send("ping".to_owned()).await {
                        return end;
                    }
                }
            }
        }
    }
}
