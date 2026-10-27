//! Coinbase Exchange feed (`wss://ws-feed.exchange.coinbase.com`): `ticker` and `heartbeat`.

use std::time::Duration;

use raki_core::{Asset, Source};
use serde::Serialize;

use super::{End, FeedCtx, Incoming, Link, Session, default_backoff, run_session};
use crate::backoff::Backoff;
use crate::parse::coinbase::parse_coinbase;

#[derive(Clone, Debug)]
pub struct CoinbaseTiming {
    /// Reconnect when no ticker arrives for this long. `heartbeat` frames (every second per
    /// product) do not count: they keep coming while the ticker channel is dead.
    pub idle_timeout: Duration,
}

impl Default for CoinbaseTiming {
    fn default() -> Self {
        CoinbaseTiming {
            idle_timeout: Duration::from_secs(30),
        }
    }
}

pub async fn run_coinbase(ctx: FeedCtx, url: String, assets: Vec<Asset>) {
    run_coinbase_with(
        ctx,
        url,
        assets,
        default_backoff(),
        CoinbaseTiming::default(),
    )
    .await;
}

pub async fn run_coinbase_with(
    ctx: FeedCtx,
    url: String,
    assets: Vec<Asset>,
    backoff: Backoff,
    timing: CoinbaseTiming,
) {
    let subscribe = subscribe_frame(&assets);
    let coinbase = Coinbase {
        url,
        subscribe,
        timing,
    };
    run_session(ctx, coinbase, backoff).await;
}

struct Coinbase {
    url: String,
    subscribe: String,
    timing: CoinbaseTiming,
}

#[derive(Serialize)]
struct Subscribe {
    #[serde(rename = "type")]
    kind: &'static str,
    product_ids: Vec<&'static str>,
    channels: [&'static str; 2],
}

/// The assets plus `USDT-USD`, which converts Binance USDT prices to USD.
fn subscribe_frame(assets: &[Asset]) -> String {
    let mut product_ids: Vec<&str> = assets.iter().map(|a| a.coinbase_product()).collect();
    product_ids.push("USDT-USD");
    let frame = Subscribe {
        kind: "subscribe",
        product_ids,
        channels: ["ticker", "heartbeat"],
    };
    serde_json::to_string(&frame).expect("frame serializes")
}

impl Session for Coinbase {
    const SOURCE: Source = Source::Coinbase;

    fn url(&self) -> &str {
        &self.url
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(self.timing.idle_timeout)
    }

    async fn run(&mut self, link: &mut Link<'_>) -> End {
        // Coinbase drops connections that do not subscribe within 5 s.
        if let Err(end) = link.send(self.subscribe.clone()).await {
            return end;
        }
        loop {
            match link.recv().await {
                Incoming::Text { text, recv_ms } => {
                    link.forward(parse_coinbase(text.as_str(), recv_ms));
                }
                Incoming::End(end) => return end,
            }
        }
    }
}
