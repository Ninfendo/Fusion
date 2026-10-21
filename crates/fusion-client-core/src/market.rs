//! Market data as the panels see it.

use std::collections::BTreeSet;
use std::fmt;

use fusion_domain::{Price, Quantity};

/// The top of the book and the last trade for one instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    /// The instrument's symbol, for example `BTCUSDT`.
    pub symbol: String,
    /// Best bid.
    pub bid: Price,
    /// Best ask.
    pub ask: Price,
    /// Last trade price.
    pub last: Price,
    /// The first price of the session, for the change since the open.
    pub open: Price,
}

/// Where panels get market data: the demo market, or the view server.
pub trait MarketData {
    /// Every instrument's quote, in a fixed order.
    fn quotes(&self) -> Vec<Quote>;
    /// The symbols, in the same order.
    fn symbols(&self) -> Vec<String>;
    /// An order book, at most `depth` levels a side, if known.
    fn book(&self, symbol: &str, depth: usize) -> Option<Book>;
}

/// Where the market data comes from, and how that goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedStatus {
    /// Made-up prices, for trying the app out.
    Demo,
    /// Waiting for the view server.
    Connecting,
    /// Connected to the view server.
    Live,
    /// The connection is lost, and why; it is tried again.
    Away(String),
}

impl fmt::Display for FeedStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Demo => f.write_str("Demo data"),
            Self::Connecting => f.write_str("Connecting to the view server"),
            Self::Live => f.write_str("Live"),
            Self::Away(why) => write!(f, "Not connected: {why}"),
        }
    }
}

/// Market data the app keeps up to date, once a frame.
pub trait MarketFeed: MarketData {
    /// Brings the data up to date. `now` is the app's clock, in seconds.
    fn update(&mut self, now: f64);
    /// The order books the panels show. Books not named can be dropped.
    fn want_books(&mut self, symbols: &BTreeSet<String>);
    /// Where the data comes from, and how that goes.
    fn status(&self) -> FeedStatus;
}

/// One price level of an order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// The price.
    pub price: Price,
    /// The quantity resting at that price.
    pub quantity: Quantity,
}

/// An order book: bids best first (highest), asks best first (lowest).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    /// Bids, highest price first.
    pub bids: Vec<Level>,
    /// Asks, lowest price first.
    pub asks: Vec<Level>,
}

impl Book {
    /// The largest quantity on either side, for scaling depth bars.
    pub fn max_quantity(&self) -> Quantity {
        self.bids
            .iter()
            .chain(&self.asks)
            .map(|level| level.quantity)
            .max()
            .unwrap_or(Quantity::ZERO)
    }
}
