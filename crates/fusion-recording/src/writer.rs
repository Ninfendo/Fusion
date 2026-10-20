//! Writing a recording.

use std::fs::{self, File};
use std::path::PathBuf;
use std::time::Duration;

use fusion_domain::Timestamp;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

use crate::columns::{Batch, schema};
use crate::stamp::utc_stamp;
use crate::{Record, RecordingError};

/// Where and how a recording is written.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// The folder of the recording.
    pub dir: PathBuf,
    /// The start of every file name, for example `binance-testnet`.
    pub prefix: String,
    /// How long one file covers. A crash loses at most the open file.
    pub rotate_every: Duration,
    /// Rows kept in memory before they go to the file.
    pub batch_rows: usize,
}

impl WriterConfig {
    /// Five-minute files, rows written in batches of 4096.
    pub fn new(dir: impl Into<PathBuf>, prefix: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            prefix: prefix.into(),
            rotate_every: Duration::from_secs(300),
            batch_rows: 4096,
        }
    }
}

struct OpenFile {
    writer: ArrowWriter<File>,
    partial: PathBuf,
    done: PathBuf,
    started: Timestamp,
}

/// Appends rows to a recording.
pub struct RecordingWriter {
    config: WriterConfig,
    symbols: Vec<String>,
    batch: Batch,
    file: Option<OpenFile>,
    seq: u64,
}

impl RecordingWriter {
    /// A writer for instruments named by `symbols`: instrument `n` is
    /// `symbols[n - 1]`.
    pub fn new(config: WriterConfig, symbols: Vec<String>) -> Self {
        Self {
            config,
            symbols,
            batch: Batch::new(),
            file: None,
            seq: 0,
        }
    }

    /// Rows written so far.
    pub fn rows(&self) -> u64 {
        self.seq
    }

    /// Appends a row received at `received`. A new file starts when the open
    /// one covers [`WriterConfig::rotate_every`].
    pub fn write(&mut self, received: Timestamp, record: &Record) -> Result<(), RecordingError> {
        let id = record.instrument().0;
        let index = usize::try_from(id)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .filter(|i| *i < self.symbols.len())
            .ok_or(RecordingError::UnknownInstrument(id))?;
        if self.file_is_full(received) {
            self.close()?;
        }
        if self.file.is_none() {
            self.open(received)?;
        }
        self.batch
            .push(self.seq, received, &self.symbols[index], record)?;
        self.seq += 1;
        if self.batch.len() >= self.config.batch_rows {
            self.flush()?;
        }
        Ok(())
    }

    fn file_is_full(&self, now: Timestamp) -> bool {
        let limit = i64::try_from(self.config.rotate_every.as_nanos()).unwrap_or(i64::MAX);
        self.file
            .as_ref()
            .is_some_and(|f| now.unix_nanos().saturating_sub(f.started.unix_nanos()) >= limit)
    }

    fn open(&mut self, started: Timestamp) -> Result<(), RecordingError> {
        fs::create_dir_all(&self.config.dir)?;
        let name = format!("{}-{}", self.config.prefix, utc_stamp(started));
        let done = self.unused_path(&name);
        let partial = done.with_extension("parquet.partial");
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
            .build();
        let writer = ArrowWriter::try_new(File::create(&partial)?, schema(), Some(props))?;
        self.file = Some(OpenFile {
            writer,
            partial,
            done,
            started,
        });
        Ok(())
    }

    /// `<dir>/<name>.parquet`, or `<name>-2.parquet` and so on if taken.
    /// The search stops at 9999: a recorder never starts that many files in
    /// one second.
    fn unused_path(&self, name: &str) -> PathBuf {
        let taken =
            |path: &PathBuf| path.exists() || path.with_extension("parquet.partial").exists();
        let first = self.config.dir.join(format!("{name}.parquet"));
        if !taken(&first) {
            return first;
        }
        (2u32..=9999)
            .map(|n| self.config.dir.join(format!("{name}-{n}.parquet")))
            .find(|path| !taken(path))
            .unwrap_or(first)
    }

    /// Writes the rows held in memory to the open file.
    pub fn flush(&mut self) -> Result<(), RecordingError> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        // An empty batch writes nothing.
        file.writer.write(&self.batch.finish()?)?;
        file.writer.flush()?;
        Ok(())
    }

    /// Completes the open file and gives it its real name. Returns that
    /// name, or `None` if no file was open.
    pub fn close(&mut self) -> Result<Option<PathBuf>, RecordingError> {
        self.flush()?;
        let Some(file) = self.file.take() else {
            return Ok(None);
        };
        file.writer.close()?;
        fs::rename(&file.partial, &file.done)?;
        Ok(Some(file.done))
    }
}
