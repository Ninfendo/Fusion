//! Binance's JSON messages, read into Fusion's market events.
//!
//! The numbers arrive as text such as `"82731.35000000"` and are read
//! straight into exact decimals. The text is borrowed from the message, so
//! reading a message allocates only the lists of levels.

use fusion_domain::{
    BookDelta, BookSnapshot, InstrumentId, Level, MarketEvent, Price, Quantity, Side, Timestamp,
    TopOfBook, Trade,
};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::symbols::SymbolTable;

/// Why a message could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Not the JSON shape Binance sends.
    #[error("not a Binance message: {0}")]
    Json(String),
    /// A stream this feed did not ask for.
    #[error("unknown stream {0}")]
    UnknownStream(String),
    /// A number that is not a decimal or a time.
    #[error("not a number: {0}")]
    Number(String),
}

impl From<serde_json::Error> for ParseError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err.to_string())
    }
}

/// A message from the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Market data.
    Event(MarketEvent),
    /// The server is about to close the connection: connect again.
    Shutdown,
}

#[derive(Deserialize)]
struct Combined<'a> {
    #[serde(borrow)]
    stream: &'a str,
    #[serde(borrow)]
    data: &'a RawValue,
}

#[derive(Deserialize)]
struct AggTradeWire<'a> {
    #[serde(rename = "a")]
    id: u64,
    #[serde(rename = "p", borrow)]
    price: &'a str,
    #[serde(rename = "q", borrow)]
    quantity: &'a str,
    #[serde(rename = "T")]
    time: i64,
    #[serde(rename = "m")]
    buyer_is_maker: bool,
}

#[derive(Deserialize)]
struct DepthWire<'a> {
    #[serde(rename = "E")]
    time: i64,
    #[serde(rename = "U")]
    first: u64,
    #[serde(rename = "u")]
    last: u64,
    #[serde(rename = "b", borrow)]
    bids: Vec<[&'a str; 2]>,
    #[serde(rename = "a", borrow)]
    asks: Vec<[&'a str; 2]>,
}

#[derive(Deserialize)]
struct BookTickerWire<'a> {
    #[serde(rename = "u")]
    update_id: u64,
    #[serde(rename = "b", borrow)]
    bid: &'a str,
    #[serde(rename = "B", borrow)]
    bid_quantity: &'a str,
    #[serde(rename = "a", borrow)]
    ask: &'a str,
    #[serde(rename = "A", borrow)]
    ask_quantity: &'a str,
}

#[derive(Deserialize)]
struct SnapshotWire<'a> {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(borrow)]
    bids: Vec<[&'a str; 2]>,
    #[serde(borrow)]
    asks: Vec<[&'a str; 2]>,
}

fn decimal<T: std::str::FromStr>(text: &str) -> Result<T, ParseError> {
    text.parse()
        .map_err(|_| ParseError::Number(text.to_owned()))
}

fn millis(value: i64) -> Result<Timestamp, ParseError> {
    Timestamp::from_unix_millis(value).ok_or_else(|| ParseError::Number(value.to_string()))
}

fn levels(raw: &[[&str; 2]]) -> Result<Vec<Level>, ParseError> {
    raw.iter()
        .map(|[price, quantity]| Ok(Level::new(decimal(price)?, decimal(quantity)?)))
        .collect()
}

impl AggTradeWire<'_> {
    fn into_trade(self, instrument: InstrumentId) -> Result<Trade, ParseError> {
        Ok(Trade {
            instrument,
            id: self.id,
            price: decimal::<Price>(self.price)?,
            quantity: decimal::<Quantity>(self.quantity)?,
            // The maker is the buyer, so the seller took liquidity.
            taker: if self.buyer_is_maker {
                Side::Sell
            } else {
                Side::Buy
            },
            time: millis(self.time)?,
        })
    }
}

/// Reads one message of a combined stream:
/// `{"stream": "btcusdt@aggTrade", "data": {...}}`.
pub fn parse_stream(text: &str, symbols: &SymbolTable) -> Result<Message, ParseError> {
    // Rare, and the only message without a stream name.
    if text.contains("\"serverShutdown\"") {
        return Ok(Message::Shutdown);
    }
    let combined: Combined<'_> = serde_json::from_str(text)?;
    let unknown = || ParseError::UnknownStream(combined.stream.to_owned());
    let (symbol, kind) = combined.stream.split_once('@').ok_or_else(unknown)?;
    let instrument = symbols.id(symbol).ok_or_else(unknown)?;
    let data = combined.data.get();
    let event = match kind {
        "aggTrade" => MarketEvent::Trade(
            serde_json::from_str::<AggTradeWire<'_>>(data)?.into_trade(instrument)?,
        ),
        "depth@100ms" | "depth" => MarketEvent::Delta(parse_delta(data, instrument)?),
        "bookTicker" => MarketEvent::Top(parse_top(data, instrument)?),
        _ => return Err(unknown()),
    };
    Ok(Message::Event(event))
}

fn parse_delta(data: &str, instrument: InstrumentId) -> Result<BookDelta, ParseError> {
    let wire: DepthWire<'_> = serde_json::from_str(data)?;
    Ok(BookDelta {
        instrument,
        first_update: wire.first,
        last_update: wire.last,
        time: millis(wire.time)?,
        bids: levels(&wire.bids)?,
        asks: levels(&wire.asks)?,
    })
}

fn parse_top(data: &str, instrument: InstrumentId) -> Result<TopOfBook, ParseError> {
    let wire: BookTickerWire<'_> = serde_json::from_str(data)?;
    Ok(TopOfBook {
        instrument,
        update_id: wire.update_id,
        bid: Level::new(decimal(wire.bid)?, decimal(wire.bid_quantity)?),
        ask: Level::new(decimal(wire.ask)?, decimal(wire.ask_quantity)?),
    })
}

/// Reads a REST book snapshot (`/api/v3/depth`).
pub fn parse_snapshot(text: &str, instrument: InstrumentId) -> Result<BookSnapshot, ParseError> {
    let wire: SnapshotWire<'_> = serde_json::from_str(text)?;
    Ok(BookSnapshot {
        instrument,
        update_id: wire.update_id,
        bids: levels(&wire.bids)?,
        asks: levels(&wire.asks)?,
    })
}

/// Reads a REST list of trades (`/api/v3/aggTrades`), oldest first.
pub fn parse_trades(text: &str, instrument: InstrumentId) -> Result<Vec<Trade>, ParseError> {
    let wire: Vec<AggTradeWire<'_>> = serde_json::from_str(text)?;
    wire.into_iter().map(|t| t.into_trade(instrument)).collect()
}

#[cfg(test)]
mod tests;
