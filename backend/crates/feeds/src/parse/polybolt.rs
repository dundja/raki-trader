//! PolyBolt (`wss://ws-live-v2.polymarket.com/ws`) frames.

use std::borrow::Cow;
use std::collections::HashMap;

use raki_core::{FeedEvent, OracleKind, Source};
use serde::Deserialize;

use super::{OraclePayload, ParseError};

/// What one PolyBolt text frame means.
#[derive(Debug, PartialEq)]
pub enum PolyboltFrame {
    Events(Vec<FeedEvent>),
    Authed,
    Subscribed {
        channel: String,
    },
    Error {
        code: String,
    },
    Pong,
    /// A valid frame we do not act on (`unsubscribed`, other channels).
    Other,
    /// Never returned by [`parse_polybolt`], which is stateless. A runner builds it when
    /// [`PolyboltSeq::check`] reports a gap.
    SeqGap {
        channel: String,
        expected: u64,
        got: u64,
    },
}

#[derive(Deserialize)]
struct Frame<'a> {
    #[serde(borrow)]
    op: Option<Cow<'a, str>>,
    #[serde(borrow)]
    channel: Option<Cow<'a, str>>,
    #[serde(borrow)]
    code: Option<Cow<'a, str>>,
    #[serde(borrow)]
    payload: Option<OraclePayload<'a>>,
}

/// Parses one PolyBolt text frame. Data frames have `channel` + `payload`; control frames have `op`.
pub fn parse_polybolt(text: &str, _recv_ms: i64) -> Result<PolyboltFrame, ParseError> {
    let frame: Frame = serde_json::from_str(text)?;
    if let Some(op) = frame.op.as_deref() {
        return Ok(match op {
            "authed" => PolyboltFrame::Authed,
            "subscribed" => PolyboltFrame::Subscribed {
                channel: frame.channel.unwrap_or_default().into_owned(),
            },
            "error" => PolyboltFrame::Error {
                code: frame.code.unwrap_or_default().into_owned(),
            },
            "pong" => PolyboltFrame::Pong,
            _ => PolyboltFrame::Other,
        });
    }
    let kind = match frame.channel.as_deref() {
        Some("price.crypto") => OracleKind::Chainlink,
        Some("price.crypto.twap") => OracleKind::Twap60,
        _ => return Ok(PolyboltFrame::Other),
    };
    match frame.payload {
        Some(payload) => Ok(PolyboltFrame::Events(
            payload.into_events(kind, Source::Polybolt)?,
        )),
        None => Err(ParseError::Unexpected("data frame without payload".into())),
    }
}

#[derive(Deserialize)]
struct SeqFrame<'a> {
    #[serde(borrow)]
    channel: Option<Cow<'a, str>>,
    seq: Option<u64>,
}

/// The `(channel, seq)` of a data frame, for feeding [`PolyboltSeq::check`]. `None` for control
/// frames and for anything that does not parse.
pub fn polybolt_seq(text: &str) -> Option<(String, u64)> {
    let f: SeqFrame = serde_json::from_str(text).ok()?;
    Some((f.channel?.into_owned(), f.seq?))
}

/// Per-channel `seq` tracking. Sequence numbers are consecutive within a channel, and a snapshot
/// consumes one like any other frame.
#[derive(Debug, Default)]
pub struct PolyboltSeq {
    last: HashMap<String, u64>,
}

impl PolyboltSeq {
    /// Records `seq` for `channel`. Returns `Some((expected, got))` if it is not the successor of
    /// the previous one. The first frame seen on a channel is never a gap.
    pub fn check(&mut self, channel: &str, seq: u64) -> Option<(u64, u64)> {
        let prev = self.last.insert(channel.to_owned(), seq)?;
        let expected = prev + 1;
        (seq != expected).then_some((expected, seq))
    }

    /// Forgets all channels; call on every (re)connect, where `seq` restarts.
    pub fn reset(&mut self) {
        self.last.clear();
    }
}
