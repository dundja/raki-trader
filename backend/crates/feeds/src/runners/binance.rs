//! Binance spot combined stream (`<base>/stream?streams=...`): `bookTicker` and `trade`.

use std::time::Duration;

use raki_core::{Asset, Source};

use super::{End, FeedCtx, Incoming, Link, Retry, Session, default_backoff, run_session};
use crate::backoff::Backoff;
use crate::parse::binance::parse_binance;

#[derive(Clone, Debug)]
pub struct BinanceTiming {
    /// Binance cuts connections after 24 h; reconnect before that.
    pub max_lifetime: Duration,
    /// Reconnect when nothing arrives for this long (the server pings every 20 s).
    pub idle_timeout: Duration,
}

impl Default for BinanceTiming {
    fn default() -> Self {
        BinanceTiming {
            max_lifetime: Duration::from_secs(23 * 3600),
            idle_timeout: Duration::from_secs(60),
        }
    }
}

/// `base_url` is e.g. `wss://stream.binance.com:9443`. No subscribe frame is needed.
pub async fn run_binance(ctx: FeedCtx, base_url: String, assets: Vec<Asset>) {
    run_binance_with(
        ctx,
        base_url,
        assets,
        default_backoff(),
        BinanceTiming::default(),
    )
    .await;
}

pub async fn run_binance_with(
    ctx: FeedCtx,
    base_url: String,
    assets: Vec<Asset>,
    backoff: Backoff,
    timing: BinanceTiming,
) {
    let url = stream_url(&base_url, &assets);
    run_session(ctx, Binance { url, timing }, backoff).await;
}

fn stream_url(base_url: &str, assets: &[Asset]) -> String {
    let streams: Vec<String> = assets
        .iter()
        .flat_map(|a| {
            let s = a.binance_symbol();
            [format!("{s}@bookTicker"), format!("{s}@trade")]
        })
        .collect();
    format!(
        "{}/stream?streams={}",
        base_url.trim_end_matches('/'),
        streams.join("/")
    )
}

struct Binance {
    url: String,
    timing: BinanceTiming,
}

impl Session for Binance {
    const SOURCE: Source = Source::Binance;

    fn url(&self) -> &str {
        &self.url
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(self.timing.idle_timeout)
    }

    async fn run(&mut self, link: &mut Link<'_>) -> End {
        let lifetime = tokio::time::sleep(self.timing.max_lifetime);
        tokio::pin!(lifetime);
        loop {
            tokio::select! {
                incoming = link.recv() => match incoming {
                    Incoming::Text { text, recv_ms } => link.forward(parse_binance(text.as_str(), recv_ms)),
                    Incoming::End(end) => return end,
                },
                () = &mut lifetime => {
                    let end = End::close(None, "scheduled reconnect before the 24 h limit");
                    return end.retry(Retry::After(Duration::ZERO));
                }
            }
        }
    }
}
