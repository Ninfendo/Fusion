//! Market data recordings.
//!
//! A recording is a folder of Parquet files. Each row is one market event
//! (a trade, best prices, a book snapshot, a book change) or a book check:
//! the fingerprint the live book had at an update number. A replay
//! rebuilds every book from the recording and must reach the same
//! fingerprint at every check.
//!
//! - [`RecordingWriter`] appends rows and starts a new file every few
//!   minutes. A file being written is named `*.parquet.partial` and gets
//!   its real name only once complete, so a crash never leaves a half file
//!   that looks whole.
//! - [`read_file`] and [`recording_files`] read a recording back.
//! - [`Replay`] rebuilds the books, checks them, and finds missing trades.

mod columns;
mod reader;
mod replay;
mod stamp;
mod writer;

use fusion_domain::{InstrumentId, MarketEvent, Timestamp};

pub use reader::{SymbolMap, read_file, recording_files};
pub use replay::{Replay, ReplayReport, SymbolReport};
pub use stamp::utc_stamp;
pub use writer::{RecordingWriter, WriterConfig};

/// The fingerprint a live book had at one update number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookCheck {
    /// The instrument.
    pub instrument: InstrumentId,
    /// The book's update number.
    pub update_id: u64,
    /// [`fusion_domain::OrderBook::digest`] at that number.
    pub digest: u64,
}

/// One row of a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// Market data.
    Event(MarketEvent),
    /// A book check.
    Check(BookCheck),
}

impl Record {
    /// The instrument the row is about.
    pub fn instrument(&self) -> InstrumentId {
        match self {
            Self::Event(event) => event.instrument(),
            Self::Check(check) => check.instrument,
        }
    }
}

/// A row as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The row's number in its recording session.
    pub seq: u64,
    /// When the recorder received it.
    pub received: Timestamp,
    /// The row.
    pub record: Record,
}

/// Why a recording could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    /// The file system refused.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Parquet refused.
    #[error("{0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// Arrow refused.
    #[error("{0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    /// A number with more decimals than a recording keeps.
    #[error("{0} has more than {SCALE} decimals")]
    TooPrecise(fusion_domain::Decimal),
    /// An instrument the writer has no symbol for.
    #[error("no symbol for instrument {0}")]
    UnknownInstrument(u32),
    /// A file that is not a Fusion recording.
    #[error("not a recording: {0}")]
    NotARecording(String),
}

/// The decimals a recording keeps for prices and quantities.
pub const SCALE: i8 = 12;
