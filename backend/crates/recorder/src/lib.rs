//! Lossless raw-frame recorder: zstd NDJSON writer and reader.

pub mod reader;
pub mod record;
pub mod task;
pub mod writer;

pub use reader::{ReadOutcome, read_file};
pub use record::{Record, RecordKind};
pub use task::{RecordTx, RecorderStats, record_channel, spawn_recorder};
pub use writer::RawWriter;
