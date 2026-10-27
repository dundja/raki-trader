//! PolyBolt (`wss://ws-live-v2.polymarket.com/ws`): Chainlink price and 60 s TWAP. Needs the
//! CLOB L2 credentials.

use std::time::Duration;

use raki_core::{Asset, Source};
use raki_recorder::RecordKind;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tokio::time::Instant;

use super::{
    End, FeedCtx, Incoming, Link, Retry, Session, default_backoff, heartbeat, run_session,
};
use crate::backoff::Backoff;
use crate::parse::polybolt::{PolyboltFrame, PolyboltSeq, parse_polybolt, polybolt_seq};

/// What the `Send` record holds in place of the auth frame.
const REDACTED_AUTH: &str = r#"{"op":"auth","rid":"a1","auth":"<redacted>"}"#;

/// CLOB L2 API credentials. `Debug` prints them redacted.
#[derive(Debug)]
pub struct PolyCreds {
    pub api_key: SecretString,
    pub secret: SecretString,
    pub passphrase: SecretString,
}

impl PolyCreds {
    /// From `POLY_API_KEY`, `POLY_API_SECRET` and `POLY_API_PASSPHRASE`; `None` unless all three
    /// are set and non-empty.
    pub fn from_env() -> Option<PolyCreds> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Option<PolyCreds> {
        let var = |name| {
            get(name)
                .filter(|v| !v.trim().is_empty())
                .map(SecretString::from)
        };
        Some(PolyCreds {
            api_key: var("POLY_API_KEY")?,
            secret: var("POLY_API_SECRET")?,
            passphrase: var("POLY_API_PASSPHRASE")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PolyboltTiming {
    /// Application ping (`{"op":"ping"}`) interval.
    pub ping: Duration,
    /// How long to wait for `authed`.
    pub auth_timeout: Duration,
    /// Reconnect when no price data arrives for this long. Pongs, acks and the server's
    /// protocol pings do not count: they keep coming while a subscription is dead.
    pub idle_timeout: Duration,
}

impl Default for PolyboltTiming {
    fn default() -> Self {
        PolyboltTiming {
            ping: Duration::from_secs(15),
            auth_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(10),
        }
    }
}

pub async fn run_polybolt(ctx: FeedCtx, url: String, creds: PolyCreds, assets: Vec<Asset>) {
    let timing = PolyboltTiming::default();
    run_polybolt_with(ctx, url, creds, assets, default_backoff(), timing).await;
}

pub async fn run_polybolt_with(
    ctx: FeedCtx,
    url: String,
    creds: PolyCreds,
    assets: Vec<Asset>,
    backoff: Backoff,
    timing: PolyboltTiming,
) {
    let subscribe = subscribe_frame(&assets);
    let polybolt = Polybolt {
        url,
        creds,
        subscribe,
        timing,
    };
    run_session(ctx, polybolt, backoff).await;
}

struct Polybolt {
    url: String,
    creds: PolyCreds,
    subscribe: String,
    timing: PolyboltTiming,
}

#[derive(Serialize)]
struct Auth<'a> {
    op: &'static str,
    rid: &'static str,
    auth: AuthCreds<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthCreds<'a> {
    api_key: &'a str,
    secret: &'a str,
    passphrase: &'a str,
}

#[derive(Serialize)]
struct Subscribe {
    op: &'static str,
    rid: &'static str,
    subscriptions: Vec<Subscription>,
}

#[derive(Serialize)]
struct Subscription {
    channel: &'static str,
    filter: Filter,
}

#[derive(Serialize)]
struct Filter {
    symbol: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_seconds: Option<u32>,
}

/// One batch frame: `price.crypto` and the 60 s `price.crypto.twap` for every asset.
fn subscribe_frame(assets: &[Asset]) -> String {
    let subscriptions = assets
        .iter()
        .flat_map(|a| {
            let symbol = a.polybolt_symbol();
            [
                Subscription {
                    channel: "price.crypto",
                    filter: Filter {
                        symbol,
                        window_seconds: None,
                    },
                },
                Subscription {
                    channel: "price.crypto.twap",
                    filter: Filter {
                        symbol,
                        window_seconds: Some(60),
                    },
                },
            ]
        })
        .collect();
    let frame = Subscribe {
        op: "subscribe",
        rid: "s1",
        subscriptions,
    };
    serde_json::to_string(&frame).expect("frame serializes")
}

impl Polybolt {
    fn auth_frame(&self) -> String {
        let frame = Auth {
            op: "auth",
            rid: "a1",
            auth: AuthCreds {
                api_key: self.creds.api_key.expose_secret(),
                secret: self.creds.secret.expose_secret(),
                passphrase: self.creds.passphrase.expose_secret(),
            },
        };
        serde_json::to_string(&frame).expect("frame serializes")
    }

    /// Waits for `authed`. Anything that arrives meanwhile is recorded but not forwarded.
    async fn await_authed(&self, link: &mut Link<'_>) -> Result<(), End> {
        let deadline = Instant::now() + self.timing.auth_timeout;
        loop {
            let incoming = tokio::time::timeout_at(deadline, link.recv())
                .await
                .map_err(|_| {
                    End::error(format!("no authed within {:?}", self.timing.auth_timeout))
                })?;
            let (text, recv_ms) = match incoming {
                Incoming::Text { text, recv_ms } => (text, recv_ms),
                Incoming::End(end) => return Err(on_end(end)),
            };
            match parse_polybolt(text.as_str(), recv_ms) {
                Ok(PolyboltFrame::Authed) => return Ok(()),
                Ok(PolyboltFrame::Error { code }) if code == "auth_invalid" => {
                    tracing::error!(
                        "PolyBolt rejected the credentials (auth_invalid); retrying at the max backoff"
                    );
                    return Err(End::error(code).retry(Retry::Max));
                }
                Ok(PolyboltFrame::Error { code }) => {
                    return Err(End::error(format!("auth failed: {code}")));
                }
                Ok(_) => {}
                Err(e) => link.parse_error(&e),
            }
        }
    }

    /// Handles one frame after the subscribe. An error frame ends the connection: the server
    /// keeps the socket open after `bad_filter`, `sub_limit`, `rate_limited`, `auth_expired`, ...,
    /// but the subscription it refused would never deliver, so reconnect (with backoff) and
    /// subscribe again.
    fn on_text(
        &self,
        link: &mut Link<'_>,
        seq: &mut PolyboltSeq,
        text: &str,
        recv_ms: i64,
    ) -> Option<End> {
        if let Some((channel, n)) = polybolt_seq(text)
            && let Some((expected, got)) = seq.check(&channel, n)
        {
            tracing::warn!(%channel, expected, got, "PolyBolt sequence gap");
            let gap = serde_json::json!({
                "source": Source::Polybolt.as_str(),
                "reason": "seq_gap",
                "channel": channel,
                "expected": expected,
                "got": got,
            });
            link.record(RecordKind::Gap, gap.to_string());
        }
        match parse_polybolt(text, recv_ms) {
            Ok(PolyboltFrame::Events(events)) => link.forward(Ok(events)),
            Ok(PolyboltFrame::Error { code }) => {
                tracing::warn!(%code, "PolyBolt error frame; reconnecting");
                let end = End::error(format!("error frame: {code}"));
                return Some(if code == "auth_invalid" {
                    end.retry(Retry::Max)
                } else {
                    end
                });
            }
            Ok(_) => {}
            Err(e) => link.parse_error(&e),
        }
        None
    }
}

/// Close 4001 (auth failed) and 4008 (policy violation) are not fixed by retrying soon.
fn on_end(end: End) -> End {
    match end.close_code() {
        Some(code @ (4001 | 4008)) => {
            tracing::error!(
                code,
                "PolyBolt closed the connection; retrying at the max backoff"
            );
            end.retry(Retry::Max)
        }
        _ => end,
    }
}

impl Session for Polybolt {
    const SOURCE: Source = Source::Polybolt;

    fn url(&self) -> &str {
        &self.url
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(self.timing.idle_timeout)
    }

    async fn run(&mut self, link: &mut Link<'_>) -> End {
        let sent = link
            .send_recorded_as(self.auth_frame(), REDACTED_AUTH.to_owned())
            .await;
        if let Err(end) = sent {
            return end;
        }
        if let Err(end) = self.await_authed(link).await {
            return end;
        }
        if let Err(end) = link.send(self.subscribe.clone()).await {
            return end;
        }
        // `seq` restarts on every connection.
        let mut seq = PolyboltSeq::default();
        let mut ping = heartbeat(self.timing.ping);
        let mut pings: u64 = 0;
        loop {
            tokio::select! {
                incoming = link.recv() => match incoming {
                    Incoming::Text { text, recv_ms } => {
                        if let Some(end) = self.on_text(link, &mut seq, text.as_str(), recv_ms) {
                            return end;
                        }
                    }
                    Incoming::End(end) => return on_end(end),
                },
                _ = ping.tick() => {
                    pings += 1;
                    let frame = format!(r#"{{"op":"ping","rid":"p{pings}"}}"#);
                    if let Err(end) = link.send(frame).await {
                        return end;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    const ALL: [(&str, &str); 3] = [
        ("POLY_API_KEY", "key-1"),
        ("POLY_API_SECRET", "c2VjcmV0"),
        ("POLY_API_PASSPHRASE", "pass-1"),
    ];

    #[test]
    fn creds_need_all_three_non_empty_vars() {
        let creds = PolyCreds::from_lookup(lookup(&ALL)).unwrap();
        assert_eq!(creds.api_key.expose_secret(), "key-1");
        assert_eq!(creds.secret.expose_secret(), "c2VjcmV0");
        assert_eq!(creds.passphrase.expose_secret(), "pass-1");

        assert!(PolyCreds::from_lookup(lookup(&ALL[..2])).is_none());
        let blank = [ALL[0], ALL[1], ("POLY_API_PASSPHRASE", " ")];
        assert!(PolyCreds::from_lookup(lookup(&blank)).is_none());
    }

    #[test]
    fn creds_debug_is_redacted() {
        let creds = PolyCreds::from_lookup(lookup(&ALL)).unwrap();
        let dbg = format!("{creds:?}");
        for (_, value) in ALL {
            assert!(!dbg.contains(value), "{dbg}");
        }
    }
}
