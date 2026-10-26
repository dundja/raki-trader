use raki_core::Source;
use serde::{Deserialize, Serialize};

/// What a raw record holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// A received text frame, exactly as it arrived.
    Text,
    /// A connection was opened.
    Open,
    /// A connection was closed.
    Close,
    /// A connection or parse error.
    Error,
    /// A frame we sent.
    Send,
    /// A feed gap marker.
    Gap,
    SessionStart,
    SessionEnd,
    /// The machine slept (clock jump detected).
    Sleep,
    /// An HTTP response body.
    Http,
}

/// One line of the raw stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Local receive time, epoch nanoseconds (UTC).
    pub t: i64,
    /// Source of the record.
    pub s: Source,
    /// Connection sequence number within the source.
    pub c: u32,
    pub k: RecordKind,
    /// The exact text of the frame or message.
    pub d: String,
}
