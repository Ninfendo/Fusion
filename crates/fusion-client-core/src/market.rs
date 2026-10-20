//! Market data as the panels see it.

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
