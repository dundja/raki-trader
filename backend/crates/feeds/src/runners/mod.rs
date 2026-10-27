//! WebSocket feed runners.
//!
//! Every runner loops until shutdown (see [`run_session`]):
//! 1. Connect, then record `Open` (d = the URL, which never carries secrets), mark the feed
//!    connected and emit [`FeedEvent::FeedReset`].
//! 2. Send the feed's subscribe or auth frames, recording each as `Send`.
//! 3. For every text frame: record `Text` with the exact text **first**, then parse and forward
//!    the events with `try_send`. Parse errors are logged and counted, never fatal. Only frames
//!    that carry data (yield events) refresh the feed status and the idle deadline: pongs,
//!    heartbeats and acks do not, so a socket whose subscriptions stopped delivering goes stale
//!    and reconnects even while its heartbeat keeps answering.
//! 4. When the connection ends, record `Close` or `Error`, then `Gap`, mark the feed
//!    disconnected, wait for the backoff and reconnect. The backoff resets after 60 s connected.
//!    A `reconnect` broadcast (sleep/wake) drops the socket and reconnects at once.
//!
//! A sharded feed (the CLOB, one connection per asset) runs one runner per shard. Shard `k`
//! numbers its connections from `k * 1_000_000 + 1`, so `(s, c)` still names one connection in
//! the raw files, and its `FeedReset` carries the shard.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use raki_core::{Asset, FeedEvent, Source};
use raki_recorder::{Record, RecordKind, RecordTx};
use serde_json::json;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_util::sync::CancellationToken;

use crate::backoff::Backoff;
use crate::parse::ParseError;
use crate::status::FeedStatusRegistry;
use crate::ws_conn::{self, WsStream};

pub mod binance;
pub mod clob;
pub mod coinbase;
pub mod polybolt;
pub mod rtds;

pub use binance::{BinanceTiming, run_binance, run_binance_with};
pub use clob::{ClobTiming, run_clob, run_clob_with};
pub use coinbase::{CoinbaseTiming, run_coinbase, run_coinbase_with};
pub use polybolt::{PolyCreds, PolyboltTiming, run_polybolt, run_polybolt_with};
pub use rtds::{RtdsTiming, run_rtds_legacy, run_rtds_legacy_with};

/// A connect (TCP + TLS + WebSocket handshake) that takes longer than this is abandoned.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A connection that lasted this long resets the backoff.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// Repeated warnings (full event channel, parse errors) are logged at most this often.
const WARN_EVERY: Duration = Duration::from_secs(10);

/// What every runner needs from the shell.
pub struct FeedCtx {
    pub rec: RecordTx,
    pub events: mpsc::Sender<FeedEvent>,
    pub status: Arc<FeedStatusRegistry>,
    /// Fires on sleep/wake: drop the socket and reconnect now.
    pub reconnect: broadcast::Receiver<()>,
    pub shutdown: CancellationToken,
}

/// 1 s doubling to 30 s.
pub(crate) fn default_backoff() -> Backoff {
    Backoff::new(Duration::from_secs(1), Duration::from_secs(30))
}

/// How long to wait before the next connect.
#[derive(Debug)]
pub(crate) enum Retry {
    Backoff,
    /// The max backoff: retrying sooner cannot help.
    Max,
    /// A fixed delay that leaves the backoff untouched.
    After(Duration),
}

#[derive(Debug)]
pub(crate) enum Ended {
    Shutdown,
    /// The server sent a close frame, or we dropped the socket on purpose (`code: None`).
    Close {
        code: Option<u16>,
        reason: String,
    },
    Error(String),
}

/// Why a connection ended and when to reconnect.
#[derive(Debug)]
pub(crate) struct End {
    pub ended: Ended,
    pub retry: Retry,
}

impl End {
    fn shutdown() -> Self {
        End {
            ended: Ended::Shutdown,
            retry: Retry::Backoff,
        }
    }

    pub(crate) fn close(code: Option<u16>, reason: impl Into<String>) -> Self {
        End {
            ended: Ended::Close {
                code,
                reason: reason.into(),
            },
            retry: Retry::Backoff,
        }
    }

    pub(crate) fn error(err: impl Into<String>) -> Self {
        End {
            ended: Ended::Error(err.into()),
            retry: Retry::Backoff,
        }
    }

    pub(crate) fn retry(self, retry: Retry) -> Self {
        End { retry, ..self }
    }

    pub(crate) fn close_code(&self) -> Option<u16> {
        match self.ended {
            Ended::Close { code, .. } => code,
            _ => None,
        }
    }
}

pub(crate) enum Incoming {
    /// A text frame, already recorded and counted.
    Text {
        text: Utf8Bytes,
        recv_ms: i64,
    },
    End(End),
}

/// One feed's protocol on an open connection.
pub(crate) trait Session {
    const SOURCE: Source;
    /// Where to connect. It never carries secrets, so it is also the `Open` record's `d`.
    fn url(&self) -> &str;
    /// Reconnect when no data frame arrives for this long. Control frames do not count.
    fn idle_timeout(&self) -> Option<Duration>;
    /// The asset this runner covers when the feed is sharded per asset.
    fn shard(&self) -> Option<Asset> {
        None
    }
    /// Drives one open connection until it ends.
    async fn run(&mut self, link: &mut Link<'_>) -> End;
}

/// Counts repeated occurrences and says when one is due to be logged.
#[derive(Default)]
struct Throttle {
    last: Option<Instant>,
    count: u64,
}

impl Throttle {
    /// Counts one occurrence. Returns how many happened since the last report, if one is due.
    fn hit(&mut self) -> Option<u64> {
        self.count += 1;
        let now = Instant::now();
        if self.last.is_some_and(|t| now - t < WARN_EVERY) {
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.count))
    }
}

/// Runner state that outlives a single connection.
struct Runner {
    ctx: FeedCtx,
    source: Source,
    /// Connection attempt number, the `c` of every record.
    conn: u32,
    parse_errors: u64,
    parse_warn: Throttle,
    events_full: Throttle,
}

impl Runner {
    fn record(&self, k: RecordKind, d: String) {
        self.record_at(raki_core::now_ns(), k, d);
    }

    fn record_at(&self, t: i64, k: RecordKind, d: String) {
        self.ctx.rec.send(Record {
            t,
            s: self.source,
            c: self.conn,
            k,
            d,
        });
    }

    /// Never blocks: with the engine behind, events are dropped (the raw frames are recorded).
    /// Drops are counted in the feed status, which fails the safety checks while they last.
    fn emit(&mut self, ev: FeedEvent) {
        match self.ctx.events.try_send(ev) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.ctx
                    .status
                    .on_event_dropped(self.source, raki_core::now_ms());
                if let Some(dropped) = self.events_full.hit() {
                    tracing::warn!(
                        source = self.source.as_str(),
                        dropped,
                        "event channel full; dropping parsed events"
                    );
                }
            }
            // The engine is gone: we are shutting down.
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    fn connect_failed(&mut self, err: String) {
        tracing::warn!(source = self.source.as_str(), error = %err, "connect failed");
        self.record(RecordKind::Error, json!({ "error": err }).to_string());
        self.ctx.status.on_connect_failed(self.source, err);
    }

    /// Records how the connection ended. `None` means shut down.
    fn finish(&mut self, end: End) -> Option<Retry> {
        let source = self.source.as_str();
        let (reason, err) = match end.ended {
            Ended::Shutdown => {
                let d = json!({ "code": null, "reason": "shutdown" });
                self.record(RecordKind::Close, d.to_string());
                self.ctx.status.on_stopped(self.source);
                tracing::info!(source, "feed stopped");
                return None;
            }
            Ended::Close { code, reason } => {
                let d = json!({ "code": code, "reason": reason });
                self.record(RecordKind::Close, d.to_string());
                let desc = match code {
                    Some(c) => format!("close {c} {reason}").trim_end().to_owned(),
                    None => reason,
                };
                let err = code.is_some_and(|c| c != 1000).then(|| desc.clone());
                (desc, err)
            }
            Ended::Error(e) => {
                self.record(RecordKind::Error, json!({ "error": e }).to_string());
                (e.clone(), Some(e))
            }
        };
        let gap = json!({ "source": source, "reason": reason });
        self.record(RecordKind::Gap, gap.to_string());
        self.ctx.status.on_disconnected(self.source, err.clone());
        match err {
            Some(error) => tracing::warn!(source, %error, "feed disconnected"),
            None => tracing::info!(source, %reason, "feed disconnected"),
        }
        Some(end.retry)
    }
}

/// One open connection, as a session sees it.
pub(crate) struct Link<'a> {
    ws: WsStream,
    rt: &'a mut Runner,
    /// Reconnect when no data frame arrives for this long.
    idle: Option<Duration>,
    last_data: Instant,
    /// Receive time of the latest text frame.
    last_recv_ms: i64,
    texts: u64,
}

impl Link<'_> {
    /// Text frames received on this connection.
    pub(crate) fn texts(&self) -> u64 {
        self.texts
    }

    /// Changes the data-idle deadline, counting from now.
    pub(crate) fn set_idle_timeout(&mut self, idle: Option<Duration>) {
        self.idle = idle;
        self.last_data = Instant::now();
    }

    /// The latest text frame carried data: the feed is fresh and the idle deadline restarts.
    pub(crate) fn mark_data(&mut self) {
        self.last_data = Instant::now();
        self.rt
            .ctx
            .status
            .on_message(self.rt.source, self.last_recv_ms);
    }

    pub(crate) fn record(&self, k: RecordKind, d: String) {
        self.rt.record(k, d);
    }

    /// Records the frame as `Send`, then sends it.
    pub(crate) async fn send(&mut self, text: String) -> Result<(), End> {
        let recorded = text.clone();
        self.send_recorded_as(text, recorded).await
    }

    /// Sends `text` but records `recorded` instead, for frames that carry secrets.
    pub(crate) async fn send_recorded_as(
        &mut self,
        text: String,
        recorded: String,
    ) -> Result<(), End> {
        self.rt.record(RecordKind::Send, recorded);
        self.ws
            .send(Message::text(text))
            .await
            .map_err(|e| End::error(format!("send failed: {e}")))
    }

    /// The next text frame, or why the connection ended. The frame is recorded before it is
    /// returned. WebSocket control frames are absorbed (tungstenite answers pings). Whether a text
    /// frame is data is up to the session ([`Link::forward`] / [`Link::mark_data`]).
    /// Cancel-safe.
    pub(crate) async fn recv(&mut self) -> Incoming {
        loop {
            let idle_deadline = self.idle.map(|d| self.last_data + d);
            let msg = tokio::select! {
                biased;
                () = self.rt.ctx.shutdown.cancelled() => return Incoming::End(End::shutdown()),
                () = reconnect_signal(&mut self.rt.ctx.reconnect) => {
                    let end = End::close(None, "forced reconnect");
                    return Incoming::End(end.retry(Retry::After(Duration::ZERO)));
                }
                msg = self.ws.next() => msg,
                () = sleep_until_opt(idle_deadline) => {
                    let idle = self.idle.unwrap_or_default();
                    return Incoming::End(End::error(format!("no data for {idle:?}")));
                }
            };
            match msg {
                Some(Ok(Message::Text(text))) => {
                    let recv_ns = raki_core::now_ns();
                    self.rt
                        .record_at(recv_ns, RecordKind::Text, text.as_str().to_owned());
                    let recv_ms = recv_ns / 1_000_000;
                    self.last_recv_ms = recv_ms;
                    self.texts += 1;
                    return Incoming::Text { text, recv_ms };
                }
                Some(Ok(Message::Close(frame))) => {
                    let end = match frame {
                        Some(f) => End::close(Some(u16::from(f.code)), f.reason.as_str()),
                        None => End::close(None, "close without status"),
                    };
                    return Incoming::End(end);
                }
                Some(Ok(Message::Binary(b))) => {
                    tracing::debug!(
                        source = self.rt.source.as_str(),
                        len = b.len(),
                        "ignoring binary frame"
                    );
                }
                Some(Ok(_)) => {} // ping, pong, raw frame
                Some(Err(e)) => return Incoming::End(End::error(e.to_string())),
                None => return Incoming::End(End::error("stream ended")),
            }
        }
    }

    /// Forwards parsed events, or logs and counts the parse error. A frame that yields events is
    /// data ([`Link::mark_data`]); one that yields none (a pong, a heartbeat, an ack, an untracked
    /// symbol) is not.
    pub(crate) fn forward(&mut self, parsed: Result<Vec<FeedEvent>, ParseError>) {
        match parsed {
            Ok(events) => {
                if !events.is_empty() {
                    self.mark_data();
                }
                events.into_iter().for_each(|ev| self.rt.emit(ev));
            }
            Err(e) => self.parse_error(&e),
        }
    }

    pub(crate) fn parse_error(&mut self, e: &ParseError) {
        self.rt.parse_errors += 1;
        if let Some(since_last) = self.rt.parse_warn.hit() {
            tracing::warn!(
                source = self.rt.source.as_str(),
                error = %e,
                since_last,
                total = self.rt.parse_errors,
                "unparsable frame (the raw frame is recorded)"
            );
        }
    }
}

/// Resolves on a reconnect request. A lagged receiver still means "reconnect"; a closed channel
/// never fires.
async fn reconnect_signal(rx: &mut broadcast::Receiver<()>) {
    match rx.recv().await {
        Ok(()) | Err(RecvError::Lagged(_)) => {}
        Err(RecvError::Closed) => std::future::pending().await,
    }
}

/// Discards reconnect requests that a fresh connection already satisfies.
fn drain_reconnects(rx: &mut broadcast::Receiver<()>) {
    while let Ok(()) | Err(TryRecvError::Lagged(_)) = rx.try_recv() {}
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

/// A ticker whose first tick is one `period` from now.
pub(crate) fn heartbeat(period: Duration) -> Interval {
    let mut i = tokio::time::interval_at(Instant::now() + period, period);
    i.set_missed_tick_behavior(MissedTickBehavior::Delay);
    i
}

/// The connect/record/backoff loop shared by every runner. Returns on shutdown.
pub(crate) async fn run_session<S: Session>(ctx: FeedCtx, mut session: S, mut backoff: Backoff) {
    let shard = session.shard();
    // Shard k numbers its connections from k * 1_000_000 + 1.
    let conn_base = shard.map_or(0, |a| {
        let k = Asset::ALL.iter().position(|&x| x == a).unwrap_or(0) as u32;
        (k + 1) * 1_000_000
    });
    let mut rt = Runner {
        ctx,
        source: S::SOURCE,
        conn: conn_base,
        parse_errors: 0,
        parse_warn: Throttle::default(),
        events_full: Throttle::default(),
    };
    loop {
        rt.conn = rt.conn.wrapping_add(1);
        let url = session.url().to_owned();
        let connected = tokio::select! {
            biased;
            () = rt.ctx.shutdown.cancelled() => return,
            r = tokio::time::timeout(CONNECT_TIMEOUT, ws_conn::connect(&url)) => r,
        };
        let retry = match connected {
            Ok(Ok(ws)) => {
                drain_reconnects(&mut rt.ctx.reconnect);
                rt.record(RecordKind::Open, url);
                rt.ctx.status.on_connected(S::SOURCE);
                rt.emit(FeedEvent::FeedReset {
                    source: S::SOURCE,
                    shard,
                });
                tracing::info!(
                    source = S::SOURCE.as_str(),
                    shard = shard.map(Asset::as_str),
                    conn = rt.conn,
                    "feed connected"
                );
                let since = Instant::now();
                let mut link = Link {
                    ws,
                    rt: &mut rt,
                    idle: session.idle_timeout(),
                    last_data: since,
                    last_recv_ms: 0,
                    texts: 0,
                };
                let end = session.run(&mut link).await;
                drop(link);
                if since.elapsed() >= STABLE_AFTER {
                    backoff.reset();
                }
                match rt.finish(end) {
                    Some(retry) => retry,
                    None => return,
                }
            }
            Ok(Err(e)) => {
                rt.connect_failed(e.to_string());
                Retry::Backoff
            }
            Err(_) => {
                rt.connect_failed(format!("connect timed out after {CONNECT_TIMEOUT:?}"));
                Retry::Backoff
            }
        };
        let delay = match retry {
            Retry::Backoff => backoff.next_delay(),
            Retry::Max => backoff.max_delay(),
            Retry::After(d) => d,
        };
        tokio::select! {
            biased;
            () = rt.ctx.shutdown.cancelled() => return,
            () = reconnect_signal(&mut rt.ctx.reconnect) => {}
            () = tokio::time::sleep(delay) => {}
        }
    }
}
