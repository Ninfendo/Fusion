//! Reading a recording back.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use arrow_array::cast::AsArray as _;
use arrow_array::types::{Decimal128Type, Int8Type, TimestampNanosecondType, UInt64Type};
use arrow_array::{Array, ArrayRef, RecordBatch};
use fusion_domain::{
    BookDelta, BookSnapshot, InstrumentId, Level, MarketEvent, Price, Quantity, Side, Timestamp,
    TopOfBook, Trade,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::columns::{CHECK, DELTA, SNAPSHOT, TOP, TRADE, col, from_scaled, schema};
use crate::{BookCheck, Record, RecordingError, Row};

/// The finished files of a recording, in time order. Files still being
/// written (`*.parquet.partial`) are left out.
pub fn recording_files(dir: &Path) -> Result<Vec<PathBuf>, RecordingError> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
        .collect();
    files.sort();
    Ok(files)
}

/// Instrument ids for symbols, given in the order symbols are first seen.
#[derive(Debug, Clone, Default)]
pub struct SymbolMap {
    symbols: Vec<String>,
}

impl SymbolMap {
    /// The id of a symbol, given a new one if it is new.
    pub fn id(&mut self, symbol: &str) -> InstrumentId {
        let index = self
            .symbols
            .iter()
            .position(|s| s == symbol)
            .unwrap_or_else(|| {
                self.symbols.push(symbol.to_owned());
                self.symbols.len() - 1
            });
        InstrumentId(u32::try_from(index + 1).unwrap_or(u32::MAX))
    }

    /// The symbol of an id.
    pub fn symbol(&self, id: InstrumentId) -> Option<&str> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        self.symbols.get(index).map(String::as_str)
    }

    /// Every symbol, by id.
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }
}

/// Every row of one recording file, in order.
pub fn read_file(path: &Path, symbols: &mut SymbolMap) -> Result<Vec<Row>, RecordingError> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)?.build()?;
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        if batch.schema() != schema() {
            return Err(RecordingError::NotARecording(path.display().to_string()));
        }
        Columns::of(&batch).read_into(symbols, &mut rows)?;
    }
    Ok(rows)
}

/// A batch's columns, by name.
struct Columns<'a> {
    batch: &'a RecordBatch,
}

fn bad(what: &str) -> RecordingError {
    RecordingError::NotARecording(format!("a {what} is missing"))
}

impl<'a> Columns<'a> {
    fn of(batch: &'a RecordBatch) -> Self {
        Self { batch }
    }

    fn column(&self, name: &str) -> &ArrayRef {
        // The schema was checked, so every column is there.
        self.batch
            .column_by_name(name)
            .unwrap_or_else(|| self.batch.column(0))
    }

    fn u64(&self, name: &str, row: usize) -> Option<u64> {
        let array = self.column(name).as_primitive::<UInt64Type>();
        array.is_valid(row).then(|| array.value(row))
    }

    fn time(&self, name: &str, row: usize) -> Option<Timestamp> {
        let array = self.column(name).as_primitive::<TimestampNanosecondType>();
        array
            .is_valid(row)
            .then(|| Timestamp::from_unix_nanos(array.value(row)))
    }

    fn text(&self, name: &str, row: usize) -> Option<&'a str> {
        let column = self.batch.column_by_name(name)?;
        let dictionary = column.as_dictionary::<Int8Type>();
        let key = dictionary.key(row)?;
        Some(dictionary.values().as_string::<i32>().value(key))
    }

    fn decimal(&self, name: &str, row: usize) -> Option<fusion_domain::Decimal> {
        let array = self.column(name).as_primitive::<Decimal128Type>();
        array.is_valid(row).then(|| from_scaled(array.value(row)))
    }

    fn level(&self, price: &str, quantity: &str, row: usize) -> Option<Level> {
        Some(Level::new(
            Price::new(self.decimal(price, row)?),
            Quantity::new(self.decimal(quantity, row)?),
        ))
    }

    fn levels(&self, prices: &str, quantities: &str, row: usize) -> Option<Vec<Level>> {
        let prices = self.column(prices).as_list::<i32>();
        let quantities = self.column(quantities).as_list::<i32>();
        // The writer fills or leaves out both lists together.
        if prices.is_null(row) {
            return None;
        }
        let p = prices.value(row);
        let q = quantities.value(row);
        let (p, q) = (
            p.as_primitive::<Decimal128Type>(),
            q.as_primitive::<Decimal128Type>(),
        );
        Some(
            (0..p.len().min(q.len()))
                .map(|i| {
                    Level::new(
                        Price::new(from_scaled(p.value(i))),
                        Quantity::new(from_scaled(q.value(i))),
                    )
                })
                .collect(),
        )
    }

    fn read_into(
        &self,
        symbols: &mut SymbolMap,
        rows: &mut Vec<Row>,
    ) -> Result<(), RecordingError> {
        for row in 0..self.batch.num_rows() {
            let symbol = self.text(col::SYMBOL, row).ok_or_else(|| bad("symbol"))?;
            let instrument = symbols.id(symbol);
            rows.push(Row {
                seq: self.u64(col::SEQ, row).ok_or_else(|| bad("seq"))?,
                received: self.time(col::RECEIVED, row).ok_or_else(|| bad("time"))?,
                record: self.record(instrument, row)?,
            });
        }
        Ok(())
    }

    fn record(&self, instrument: InstrumentId, row: usize) -> Result<Record, RecordingError> {
        let kind = self.text(col::KIND, row).ok_or_else(|| bad("kind"))?;
        let event = |e: Option<MarketEvent>| e.map(Record::Event);
        let record = match kind {
            TRADE => event(self.trade(instrument, row).map(MarketEvent::Trade)),
            TOP => event(self.top(instrument, row).map(MarketEvent::Top)),
            SNAPSHOT => event(self.snapshot(instrument, row).map(MarketEvent::Snapshot)),
            DELTA => event(self.delta(instrument, row).map(MarketEvent::Delta)),
            CHECK => self.check(instrument, row).map(Record::Check),
            other => {
                return Err(RecordingError::NotARecording(format!(
                    "a row of kind {other}"
                )));
            }
        };
        record.ok_or_else(|| bad(kind))
    }

    fn check(&self, instrument: InstrumentId, row: usize) -> Option<BookCheck> {
        Some(BookCheck {
            instrument,
            update_id: self.u64(col::LAST_UPDATE, row)?,
            digest: self.u64(col::DIGEST, row)?,
        })
    }

    fn trade(&self, instrument: InstrumentId, row: usize) -> Option<Trade> {
        Some(Trade {
            instrument,
            id: self.u64(col::TRADE_ID, row)?,
            price: Price::new(self.decimal(col::PRICE, row)?),
            quantity: Quantity::new(self.decimal(col::QUANTITY, row)?),
            taker: match self.text(col::TAKER, row)? {
                "buy" => Side::Buy,
                "sell" => Side::Sell,
                _ => return None,
            },
            time: self.time(col::VENUE_TIME, row)?,
        })
    }

    fn top(&self, instrument: InstrumentId, row: usize) -> Option<TopOfBook> {
        Some(TopOfBook {
            instrument,
            update_id: self.u64(col::LAST_UPDATE, row)?,
            bid: self.level(col::BID_PRICE, col::BID_QUANTITY, row)?,
            ask: self.level(col::ASK_PRICE, col::ASK_QUANTITY, row)?,
        })
    }

    fn snapshot(&self, instrument: InstrumentId, row: usize) -> Option<BookSnapshot> {
        Some(BookSnapshot {
            instrument,
            update_id: self.u64(col::LAST_UPDATE, row)?,
            bids: self.levels(col::BID_PRICES, col::BID_QUANTITIES, row)?,
            asks: self.levels(col::ASK_PRICES, col::ASK_QUANTITIES, row)?,
        })
    }

    fn delta(&self, instrument: InstrumentId, row: usize) -> Option<BookDelta> {
        Some(BookDelta {
            instrument,
            first_update: self.u64(col::FIRST_UPDATE, row)?,
            last_update: self.u64(col::LAST_UPDATE, row)?,
            time: self.time(col::VENUE_TIME, row)?,
            bids: self.levels(col::BID_PRICES, col::BID_QUANTITIES, row)?,
            asks: self.levels(col::ASK_PRICES, col::ASK_QUANTITIES, row)?,
        })
    }
}
