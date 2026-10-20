//! The columns of a recording, and the rows in and out of them.
//!
//! One wide table: every row has a kind, and only that kind's columns are
//! filled. Prices and quantities are exact decimals with [`SCALE`]
//! decimals; book levels are lists of prices and lists of quantities.

use std::sync::Arc;

use arrow_array::builder::{
    Decimal128Builder, ListBuilder, StringDictionaryBuilder, TimestampNanosecondBuilder,
    UInt64Builder,
};
use arrow_array::types::Int8Type;
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use fusion_domain::{Decimal, Level, MarketEvent, Side, Timestamp};

use crate::{BookCheck, Record, RecordingError, SCALE};

/// Decimal digits a recorded number may have in all.
const PRECISION: u8 = 38;

/// The kinds of rows.
pub(crate) const TRADE: &str = "trade";
pub(crate) const TOP: &str = "top";
pub(crate) const SNAPSHOT: &str = "snapshot";
pub(crate) const DELTA: &str = "delta";
pub(crate) const CHECK: &str = "check";

/// The column names, in order.
pub(crate) mod col {
    pub const SEQ: &str = "seq";
    pub const RECEIVED: &str = "received";
    pub const KIND: &str = "kind";
    pub const SYMBOL: &str = "symbol";
    pub const VENUE_TIME: &str = "venue_time";
    pub const TRADE_ID: &str = "trade_id";
    pub const FIRST_UPDATE: &str = "first_update";
    pub const LAST_UPDATE: &str = "last_update";
    pub const PRICE: &str = "price";
    pub const QUANTITY: &str = "quantity";
    pub const TAKER: &str = "taker";
    pub const BID_PRICE: &str = "bid_price";
    pub const BID_QUANTITY: &str = "bid_quantity";
    pub const ASK_PRICE: &str = "ask_price";
    pub const ASK_QUANTITY: &str = "ask_quantity";
    pub const BID_PRICES: &str = "bid_prices";
    pub const BID_QUANTITIES: &str = "bid_quantities";
    pub const ASK_PRICES: &str = "ask_prices";
    pub const ASK_QUANTITIES: &str = "ask_quantities";
    pub const DIGEST: &str = "digest";
}

fn decimal_type() -> DataType {
    DataType::Decimal128(PRECISION, SCALE)
}

fn list_type() -> DataType {
    DataType::List(Arc::new(Field::new_list_field(decimal_type(), true)))
}

fn dictionary_type() -> DataType {
    DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8))
}

/// The schema of every recording file.
pub(crate) fn schema() -> SchemaRef {
    let time = DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into()));
    let f = |name, data_type, nullable| Field::new(name, data_type, nullable);
    Arc::new(Schema::new(vec![
        f(col::SEQ, DataType::UInt64, false),
        f(col::RECEIVED, time.clone(), false),
        f(col::KIND, dictionary_type(), false),
        f(col::SYMBOL, dictionary_type(), false),
        f(col::VENUE_TIME, time, true),
        f(col::TRADE_ID, DataType::UInt64, true),
        f(col::FIRST_UPDATE, DataType::UInt64, true),
        f(col::LAST_UPDATE, DataType::UInt64, true),
        f(col::PRICE, decimal_type(), true),
        f(col::QUANTITY, decimal_type(), true),
        f(col::TAKER, dictionary_type(), true),
        f(col::BID_PRICE, decimal_type(), true),
        f(col::BID_QUANTITY, decimal_type(), true),
        f(col::ASK_PRICE, decimal_type(), true),
        f(col::ASK_QUANTITY, decimal_type(), true),
        f(col::BID_PRICES, list_type(), true),
        f(col::BID_QUANTITIES, list_type(), true),
        f(col::ASK_PRICES, list_type(), true),
        f(col::ASK_QUANTITIES, list_type(), true),
        f(col::DIGEST, DataType::UInt64, true),
    ]))
}

/// A decimal as the integer a recording stores: the value times 10^SCALE.
/// A value with more decimals is refused, never rounded.
pub(crate) fn to_scaled(value: Decimal) -> Result<i128, RecordingError> {
    let value = value.normalize();
    let shift = i32::from(SCALE) - i32::try_from(value.scale()).unwrap_or(i32::MAX);
    let factor = u32::try_from(shift)
        .ok()
        .and_then(|s| 10i128.checked_pow(s))
        .ok_or(RecordingError::TooPrecise(value))?;
    value
        .mantissa()
        .checked_mul(factor)
        .filter(|scaled| scaled.unsigned_abs() < 10u128.pow(u32::from(PRECISION)))
        .ok_or(RecordingError::TooPrecise(value))
}

/// The decimal a recording stores as `scaled`.
pub(crate) fn from_scaled(scaled: i128) -> Decimal {
    // SCALE is 12, well inside what a Decimal can hold.
    Decimal::from_i128_with_scale(scaled, SCALE.unsigned_abs().into())
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

/// Rows on their way into one record batch.
pub(crate) struct Batch {
    seq: UInt64Builder,
    received: TimestampNanosecondBuilder,
    kind: StringDictionaryBuilder<Int8Type>,
    symbol: StringDictionaryBuilder<Int8Type>,
    venue_time: TimestampNanosecondBuilder,
    trade_id: UInt64Builder,
    first_update: UInt64Builder,
    last_update: UInt64Builder,
    price: Decimal128Builder,
    quantity: Decimal128Builder,
    taker: StringDictionaryBuilder<Int8Type>,
    bid_price: Decimal128Builder,
    bid_quantity: Decimal128Builder,
    ask_price: Decimal128Builder,
    ask_quantity: Decimal128Builder,
    bid_prices: ListBuilder<Decimal128Builder>,
    bid_quantities: ListBuilder<Decimal128Builder>,
    ask_prices: ListBuilder<Decimal128Builder>,
    ask_quantities: ListBuilder<Decimal128Builder>,
    digest: UInt64Builder,
    rows: usize,
}

fn decimals() -> Decimal128Builder {
    Decimal128Builder::new().with_data_type(decimal_type())
}

fn decimal_lists() -> ListBuilder<Decimal128Builder> {
    ListBuilder::new(decimals())
}

fn times() -> TimestampNanosecondBuilder {
    TimestampNanosecondBuilder::new().with_timezone("UTC")
}

impl Batch {
    pub(crate) fn new() -> Self {
        Self {
            seq: UInt64Builder::new(),
            received: times(),
            kind: StringDictionaryBuilder::new(),
            symbol: StringDictionaryBuilder::new(),
            venue_time: times(),
            trade_id: UInt64Builder::new(),
            first_update: UInt64Builder::new(),
            last_update: UInt64Builder::new(),
            price: decimals(),
            quantity: decimals(),
            taker: StringDictionaryBuilder::new(),
            bid_price: decimals(),
            bid_quantity: decimals(),
            ask_price: decimals(),
            ask_quantity: decimals(),
            bid_prices: decimal_lists(),
            bid_quantities: decimal_lists(),
            ask_prices: decimal_lists(),
            ask_quantities: decimal_lists(),
            digest: UInt64Builder::new(),
            rows: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.rows
    }

    /// Adds a row. On an error nothing is added.
    pub(crate) fn push(
        &mut self,
        seq: u64,
        received: Timestamp,
        symbol: &str,
        record: &Record,
    ) -> Result<(), RecordingError> {
        let cells = Cells::of(record)?;
        self.seq.append_value(seq);
        self.received.append_value(received.unix_nanos());
        self.kind.append_value(cells.kind);
        self.symbol.append_value(symbol);
        self.venue_time
            .append_option(cells.venue_time.map(Timestamp::unix_nanos));
        self.trade_id.append_option(cells.trade_id);
        self.first_update.append_option(cells.first_update);
        self.last_update.append_option(cells.last_update);
        self.price.append_option(cells.price);
        self.quantity.append_option(cells.quantity);
        self.taker.append_option(cells.taker);
        self.bid_price.append_option(cells.bid.map(|l| l.0));
        self.bid_quantity.append_option(cells.bid.map(|l| l.1));
        self.ask_price.append_option(cells.ask.map(|l| l.0));
        self.ask_quantity.append_option(cells.ask.map(|l| l.1));
        append_levels(&mut self.bid_prices, &mut self.bid_quantities, cells.bids);
        append_levels(&mut self.ask_prices, &mut self.ask_quantities, cells.asks);
        self.digest.append_option(cells.digest);
        self.rows += 1;
        Ok(())
    }

    /// The rows so far as one record batch; the batch starts empty again.
    pub(crate) fn finish(&mut self) -> Result<RecordBatch, RecordingError> {
        let columns: Vec<ArrayRef> = vec![
            Arc::new(self.seq.finish()),
            Arc::new(self.received.finish()),
            Arc::new(self.kind.finish()),
            Arc::new(self.symbol.finish()),
            Arc::new(self.venue_time.finish()),
            Arc::new(self.trade_id.finish()),
            Arc::new(self.first_update.finish()),
            Arc::new(self.last_update.finish()),
            Arc::new(self.price.finish()),
            Arc::new(self.quantity.finish()),
            Arc::new(self.taker.finish()),
            Arc::new(self.bid_price.finish()),
            Arc::new(self.bid_quantity.finish()),
            Arc::new(self.ask_price.finish()),
            Arc::new(self.ask_quantity.finish()),
            Arc::new(self.bid_prices.finish()),
            Arc::new(self.bid_quantities.finish()),
            Arc::new(self.ask_prices.finish()),
            Arc::new(self.ask_quantities.finish()),
            Arc::new(self.digest.finish()),
        ];
        self.rows = 0;
        Ok(RecordBatch::try_new(schema(), columns)?)
    }
}

fn append_levels(
    prices: &mut ListBuilder<Decimal128Builder>,
    quantities: &mut ListBuilder<Decimal128Builder>,
    levels: Option<Vec<(i128, i128)>>,
) {
    let Some(levels) = levels else {
        prices.append_null();
        quantities.append_null();
        return;
    };
    for (price, quantity) in levels {
        prices.values().append_value(price);
        quantities.values().append_value(quantity);
    }
    prices.append(true);
    quantities.append(true);
}

/// One row's cells, already scaled; empty cells are `None`.
#[derive(Default)]
struct Cells {
    kind: &'static str,
    venue_time: Option<Timestamp>,
    trade_id: Option<u64>,
    first_update: Option<u64>,
    last_update: Option<u64>,
    price: Option<i128>,
    quantity: Option<i128>,
    taker: Option<&'static str>,
    bid: Option<(i128, i128)>,
    ask: Option<(i128, i128)>,
    bids: Option<Vec<(i128, i128)>>,
    asks: Option<Vec<(i128, i128)>>,
    digest: Option<u64>,
}

fn scaled_level(level: Level) -> Result<(i128, i128), RecordingError> {
    Ok((
        to_scaled(level.price.value())?,
        to_scaled(level.quantity.value())?,
    ))
}

fn scaled_levels(levels: &[Level]) -> Result<Vec<(i128, i128)>, RecordingError> {
    levels.iter().copied().map(scaled_level).collect()
}

impl Cells {
    fn of(record: &Record) -> Result<Self, RecordingError> {
        Ok(match record {
            Record::Event(MarketEvent::Trade(t)) => Self {
                kind: TRADE,
                venue_time: Some(t.time),
                trade_id: Some(t.id),
                price: Some(to_scaled(t.price.value())?),
                quantity: Some(to_scaled(t.quantity.value())?),
                taker: Some(side_name(t.taker)),
                ..Self::default()
            },
            Record::Event(MarketEvent::Top(t)) => Self {
                kind: TOP,
                last_update: Some(t.update_id),
                bid: Some(scaled_level(t.bid)?),
                ask: Some(scaled_level(t.ask)?),
                ..Self::default()
            },
            Record::Event(MarketEvent::Snapshot(s)) => Self {
                kind: SNAPSHOT,
                last_update: Some(s.update_id),
                bids: Some(scaled_levels(&s.bids)?),
                asks: Some(scaled_levels(&s.asks)?),
                ..Self::default()
            },
            Record::Event(MarketEvent::Delta(d)) => Self {
                kind: DELTA,
                venue_time: Some(d.time),
                first_update: Some(d.first_update),
                last_update: Some(d.last_update),
                bids: Some(scaled_levels(&d.bids)?),
                asks: Some(scaled_levels(&d.asks)?),
                ..Self::default()
            },
            Record::Check(BookCheck {
                update_id, digest, ..
            }) => Self {
                kind: CHECK,
                last_update: Some(*update_id),
                digest: Some(*digest),
                ..Self::default()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().expect("a decimal")
    }

    #[test]
    fn decimals_scale_exactly_or_are_refused() {
        assert_eq!(to_scaled(dec("1")).ok(), Some(1_000_000_000_000));
        assert_eq!(
            to_scaled(dec("82731.35000000")).ok(),
            Some(82_731_350_000_000_000)
        );
        assert_eq!(to_scaled(dec("0.000000000001")).ok(), Some(1));
        assert_eq!(to_scaled(dec("-2.5")).ok(), Some(-2_500_000_000_000));
        assert!(matches!(
            to_scaled(dec("0.0000000000001")),
            Err(RecordingError::TooPrecise(_))
        ));
        assert_eq!(from_scaled(82_731_350_000_000_000), dec("82731.35"));
        assert_eq!(from_scaled(1), dec("0.000000000001"));
    }

    #[test]
    fn numbers_too_big_for_38_digits_are_refused() {
        assert!(matches!(
            to_scaled(Decimal::MAX),
            Err(RecordingError::TooPrecise(_))
        ));
        // Fits in 128 bits, but not in 38 digits.
        let huge = Decimal::from_i128_with_scale(150_000_000_000_000_000_000_000_000, 0);
        assert!(matches!(
            to_scaled(huge),
            Err(RecordingError::TooPrecise(_))
        ));
        let edge = Decimal::from_i128_with_scale(100_000_000_000_000_000_000_000_000, 0);
        assert!(
            matches!(to_scaled(edge), Err(RecordingError::TooPrecise(_))),
            "10^38 needs 39 digits"
        );
        let largest = Decimal::from_i128_with_scale(99_999_999_999_999_999_999_999_999, 0);
        assert!(to_scaled(largest).is_ok());
    }

    #[test]
    fn a_batch_matches_the_schema() {
        let mut batch = Batch::new();
        let check = Record::Check(BookCheck {
            instrument: fusion_domain::InstrumentId(1),
            update_id: 7,
            digest: 9,
        });
        batch
            .push(0, Timestamp::UNIX_EPOCH, "BTCUSDT", &check)
            .expect("pushed");
        assert_eq!(batch.len(), 1);
        let finished = batch.finish().expect("a batch");
        assert_eq!(finished.num_rows(), 1);
        assert_eq!(finished.schema(), schema());
        assert_eq!(batch.len(), 0, "starts empty again");
    }
}
