//! Market data as tables.
//!
//! - `prices`: one row per instrument, keyed by symbol: best bid and ask,
//!   last trade, change since the first trade seen, volume and trade count.
//! - `book/<SYMBOL>`: the top [`BOOK_DEPTH`] levels of each side of one
//!   instrument's book, keyed `b00`..`b19` and `a00`..`a19`. One small table
//!   per instrument, so an L2 panel reads 40 rows, not every book at once.

use fusion_domain::{Decimal, InstrumentId, Level, MarketEvent, OrderBook, Side, TopOfBook, Trade};
use fusion_protocol::Cell;
use rust_decimal::prelude::ToPrimitive as _;

use crate::session::Tables;
use crate::table::Table;

/// The prices table.
pub const PRICES: &str = "prices";

/// Its columns.
pub const PRICE_COLUMNS: [&str; 11] = [
    "symbol", "bid", "bid_qty", "ask", "ask_qty", "last", "last_qty", "change", "volume", "trades",
    "time",
];

/// Levels kept on each side of a book table.
pub const BOOK_DEPTH: usize = 20;

/// The columns of a book table.
pub const BOOK_COLUMNS: [&str; 4] = ["side", "level", "price", "quantity"];

/// The name of an instrument's book table.
pub fn book_table(symbol: &str) -> String {
    format!("book/{symbol}")
}

/// The row key of a book level: `b00` for the best bid, `a03` for the
/// fourth ask.
pub fn book_key(side: Side, level: usize) -> String {
    let letter = match side {
        Side::Buy => 'b',
        Side::Sell => 'a',
    };
    format!("{letter}{level:02}")
}

mod col {
    pub const BID: usize = 1;
    pub const BID_QTY: usize = 2;
    pub const ASK: usize = 3;
    pub const ASK_QTY: usize = 4;
    pub const LAST: usize = 5;
    pub const LAST_QTY: usize = 6;
    pub const CHANGE: usize = 7;
    pub const VOLUME: usize = 8;
    pub const TRADES: usize = 9;
    pub const TIME: usize = 10;
    pub const PRICE: usize = 2;
    pub const QUANTITY: usize = 3;
}

struct Instrument {
    symbol: String,
    book_table: String,
    book: Option<OrderBook>,
    /// Book row keys: bids, then asks.
    keys: [Vec<String>; 2],
    first_price: Option<Decimal>,
    volume: Decimal,
    trades: i64,
}

/// Keeps the market tables up to date from market events.
pub struct MarketTables {
    instruments: Vec<Instrument>,
}

fn dec(value: Decimal) -> Cell {
    Cell::Decimal(value)
}

impl MarketTables {
    /// Creates `prices` and one book table per symbol in `tables`.
    /// Instrument `n` is `symbols[n - 1]`.
    pub fn new(symbols: &[String], tables: &mut Tables) -> Self {
        let mut prices = Table::new(PRICES, &PRICE_COLUMNS);
        let instruments = symbols
            .iter()
            .map(|symbol| {
                prices.upsert(symbol.as_str(), vec![Cell::Text(symbol.clone())]);
                let keys = [Side::Buy, Side::Sell].map(|side| {
                    (0..BOOK_DEPTH)
                        .map(|n| book_key(side, n))
                        .collect::<Vec<_>>()
                });
                let mut book = Table::new(book_table(symbol), &BOOK_COLUMNS);
                for (side, side_keys) in ["bid", "ask"].iter().zip(&keys) {
                    for (level, key) in side_keys.iter().enumerate() {
                        let level = i64::try_from(level).unwrap_or(i64::MAX);
                        book.upsert(
                            key.as_str(),
                            vec![Cell::Text((*side).into()), Cell::Integer(level)],
                        );
                    }
                }
                let instrument = Instrument {
                    symbol: symbol.clone(),
                    book_table: book.name().to_owned(),
                    book: None,
                    keys,
                    first_price: None,
                    volume: Decimal::ZERO,
                    trades: 0,
                };
                tables.insert(book);
                instrument
            })
            .collect();
        tables.insert(prices);
        Self { instruments }
    }

    fn slot(&mut self, id: InstrumentId) -> Option<&mut Instrument> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        self.instruments.get_mut(index)
    }

    /// Applies one market event to the tables.
    pub fn apply(&mut self, event: &MarketEvent, tables: &mut Tables) {
        let Some(instrument) = self.slot(event.instrument()) else {
            return;
        };
        match event {
            MarketEvent::Top(top) => instrument.top(top, tables),
            MarketEvent::Trade(trade) => instrument.trade(trade, tables),
            MarketEvent::Snapshot(snapshot) => {
                instrument.book = Some(OrderBook::from_snapshot(snapshot));
                instrument.write_book(tables);
            }
            MarketEvent::Delta(delta) => {
                let applied = instrument.book.as_mut().map(|book| book.apply(delta));
                match applied {
                    Some(Ok(_)) => instrument.write_book(tables),
                    // A gap: the book waits for the next snapshot.
                    Some(Err(_)) => instrument.book = None,
                    None => {}
                }
            }
        }
    }
}

impl Instrument {
    fn top(&self, top: &TopOfBook, tables: &mut Tables) {
        let Some(prices) = tables.get_mut(PRICES) else {
            return;
        };
        let key = self.symbol.as_str();
        prices.set(key, col::BID, dec(top.bid.price.value()));
        prices.set(key, col::BID_QTY, dec(top.bid.quantity.value()));
        prices.set(key, col::ASK, dec(top.ask.price.value()));
        prices.set(key, col::ASK_QTY, dec(top.ask.quantity.value()));
    }

    fn trade(&mut self, trade: &Trade, tables: &mut Tables) {
        let price = trade.price.value();
        let first = *self.first_price.get_or_insert(price);
        self.volume += trade.quantity.value();
        self.trades += 1;
        let change = if first.is_zero() {
            0.0
        } else {
            ((price - first) / first * Decimal::ONE_HUNDRED)
                .to_f64()
                .unwrap_or(0.0)
        };
        let Some(prices) = tables.get_mut(PRICES) else {
            return;
        };
        let key = self.symbol.as_str();
        prices.set(key, col::LAST, dec(price));
        prices.set(key, col::LAST_QTY, dec(trade.quantity.value()));
        prices.set(key, col::CHANGE, Cell::Real(change));
        prices.set(key, col::VOLUME, dec(self.volume));
        prices.set(key, col::TRADES, Cell::Integer(self.trades));
        prices.set(key, col::TIME, Cell::Time(trade.time));
    }

    fn write_book(&self, tables: &mut Tables) {
        let (Some(book), Some(table)) = (&self.book, tables.get_mut(&self.book_table)) else {
            return;
        };
        let sides: [Vec<Level>; 2] = [
            book.bids().take(BOOK_DEPTH).collect(),
            book.asks().take(BOOK_DEPTH).collect(),
        ];
        for (levels, keys) in sides.iter().zip(&self.keys) {
            for (n, key) in keys.iter().enumerate() {
                let (price, quantity) = levels.get(n).map_or((Cell::Empty, Cell::Empty), |l| {
                    (dec(l.price.value()), dec(l.quantity.value()))
                });
                table.set(key, col::PRICE, price);
                table.set(key, col::QUANTITY, quantity);
            }
        }
    }
}

#[cfg(test)]
mod tests;
