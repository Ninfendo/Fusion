//! Market data: trades, best prices and order books, as every venue adapter
//! delivers them.
//!
//! An order book starts from a [`BookSnapshot`] and then takes numbered
//! [`BookDelta`]s. Each delta covers a range of venue update numbers; a
//! missing number is a gap, and the book must start again from a fresh
//! snapshot. Trades carry numbers too, so [`TradeSequence`] can tell a
//! missed trade from a repeated one.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::fix::Side;
use crate::ids::InstrumentId;
use crate::num::{Decimal, Price, Quantity};
use crate::time::Timestamp;

/// One price level: a price and the quantity resting there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Level {
    /// The price.
    pub price: Price,
    /// The quantity. Zero in a delta means the level is gone.
    pub quantity: Quantity,
}

impl Level {
    /// A level.
    pub const fn new(price: Price, quantity: Quantity) -> Self {
        Self { price, quantity }
    }
}

/// One trade. On Binance: one aggregated trade, the fills of one taker
/// order at one price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trade {
    /// The instrument.
    pub instrument: InstrumentId,
    /// The venue's trade number. Each trade is the one before plus one.
    pub id: u64,
    /// The price.
    pub price: Price,
    /// The quantity.
    pub quantity: Quantity,
    /// The side of the order that took liquidity.
    pub taker: Side,
    /// When the venue matched it.
    pub time: Timestamp,
}

/// The best bid and the best ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopOfBook {
    /// The instrument.
    pub instrument: InstrumentId,
    /// The venue's book update number this top belongs to.
    pub update_id: u64,
    /// The best bid.
    pub bid: Level,
    /// The best ask.
    pub ask: Level,
}

/// A whole book at one update number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSnapshot {
    /// The instrument.
    pub instrument: InstrumentId,
    /// The venue's update number of the snapshot.
    pub update_id: u64,
    /// Bids, best first.
    pub bids: Vec<Level>,
    /// Asks, best first.
    pub asks: Vec<Level>,
}

/// Changes to a book, covering the venue's update numbers
/// `first_update..=last_update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookDelta {
    /// The instrument.
    pub instrument: InstrumentId,
    /// The first update number in the delta.
    pub first_update: u64,
    /// The last update number in the delta.
    pub last_update: u64,
    /// When the venue sent it.
    pub time: Timestamp,
    /// Bid levels to set; a zero quantity removes the level.
    pub bids: Vec<Level>,
    /// Ask levels to set; a zero quantity removes the level.
    pub asks: Vec<Level>,
}

/// One piece of market data, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketEvent {
    /// A trade.
    Trade(Trade),
    /// New best prices.
    Top(TopOfBook),
    /// A whole book: the start, or a new start after a gap.
    Snapshot(BookSnapshot),
    /// Changes to a book.
    Delta(BookDelta),
}

impl MarketEvent {
    /// The instrument the event is about.
    pub fn instrument(&self) -> InstrumentId {
        match self {
            Self::Trade(t) => t.instrument,
            Self::Top(t) => t.instrument,
            Self::Snapshot(s) => s.instrument,
            Self::Delta(d) => d.instrument,
        }
    }
}

/// What [`OrderBook::apply`] did with a delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaOutcome {
    /// The book took the delta.
    Applied,
    /// The book already had every update in it; nothing changed.
    Stale,
}

/// Updates are missing between the book and a delta. Start again from a
/// fresh snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookGap {
    /// The update number the book expected next.
    pub expected: u64,
    /// The first update number of the delta that came.
    pub got: u64,
}

/// An order book: every bid and ask level, at one venue update number.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrderBook {
    update_id: u64,
    bids: BTreeMap<Price, Quantity>,
    asks: BTreeMap<Price, Quantity>,
}

impl OrderBook {
    /// The book of a snapshot.
    pub fn from_snapshot(snapshot: &BookSnapshot) -> Self {
        let side = |levels: &[Level]| {
            levels
                .iter()
                .filter(|l| !l.quantity.is_zero())
                .map(|l| (l.price, l.quantity))
                .collect()
        };
        Self {
            update_id: snapshot.update_id,
            bids: side(&snapshot.bids),
            asks: side(&snapshot.asks),
        }
    }

    /// The venue update number the book is at.
    pub fn update_id(&self) -> u64 {
        self.update_id
    }

    /// Applies a delta. A delta the book already has is ignored; a delta
    /// that starts after the next update number is a gap.
    pub fn apply(&mut self, delta: &BookDelta) -> Result<DeltaOutcome, BookGap> {
        if delta.last_update <= self.update_id {
            return Ok(DeltaOutcome::Stale);
        }
        let expected = self.update_id + 1;
        if delta.first_update > expected {
            return Err(BookGap {
                expected,
                got: delta.first_update,
            });
        }
        set_levels(&mut self.bids, &delta.bids);
        set_levels(&mut self.asks, &delta.asks);
        self.update_id = delta.last_update;
        Ok(DeltaOutcome::Applied)
    }

    /// The best bid.
    pub fn best_bid(&self) -> Option<Level> {
        self.bids
            .iter()
            .next_back()
            .map(|(p, q)| Level::new(*p, *q))
    }

    /// The best ask.
    pub fn best_ask(&self) -> Option<Level> {
        self.asks.iter().next().map(|(p, q)| Level::new(*p, *q))
    }

    /// Bids, best first.
    pub fn bids(&self) -> impl Iterator<Item = Level> + '_ {
        self.bids.iter().rev().map(|(p, q)| Level::new(*p, *q))
    }

    /// Asks, best first.
    pub fn asks(&self) -> impl Iterator<Item = Level> + '_ {
        self.asks.iter().map(|(p, q)| Level::new(*p, *q))
    }

    /// The number of bid and ask levels.
    pub fn depth(&self) -> (usize, usize) {
        (self.bids.len(), self.asks.len())
    }

    /// A fingerprint of the whole book: the update number and every level.
    /// Two books with the same fingerprint are the same book. It is the
    /// same on every machine and in every build (FNV-1a over the numbers),
    /// so a recording can store it and a later replay can check it.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv::default();
        hash.write(&self.update_id.to_le_bytes());
        for (tag, side) in [(b'b', &self.bids), (b'a', &self.asks)] {
            hash.write(&[tag]);
            for (price, quantity) in side {
                hash.write(&decimal_bytes(price.value()));
                hash.write(&decimal_bytes(quantity.value()));
            }
        }
        hash.finish()
    }
}

fn set_levels(side: &mut BTreeMap<Price, Quantity>, levels: &[Level]) {
    for level in levels {
        if level.quantity.is_zero() {
            side.remove(&level.price);
        } else {
            side.insert(level.price, level.quantity);
        }
    }
}

/// A decimal's bytes after trailing zeros are dropped, so `1.50` and `1.5`
/// give the same bytes.
fn decimal_bytes(value: Decimal) -> [u8; 16] {
    value.normalize().serialize()
}

/// FNV-1a, 64 bits: tiny, and defined by its constants, so it never
/// changes between builds.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// What [`TradeSequence::observe`] made of a trade number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The first trade seen.
    First,
    /// The trade after the last one.
    Next,
    /// A trade seen before; drop it.
    Repeat,
    /// Trades are missing: these numbers. The trade itself is new.
    Gap(Range<u64>),
}

/// Follows the trade numbers of one instrument to find missed and repeated
/// trades.
#[derive(Debug, Clone, Default)]
pub struct TradeSequence {
    last: Option<u64>,
}

impl TradeSequence {
    /// The last trade number seen.
    pub fn last(&self) -> Option<u64> {
        self.last
    }

    /// Takes a trade number.
    pub fn observe(&mut self, id: u64) -> Seen {
        let Some(last) = self.last else {
            self.last = Some(id);
            return Seen::First;
        };
        if id <= last {
            return Seen::Repeat;
        }
        self.last = Some(id);
        if id == last + 1 {
            Seen::Next
        } else {
            Seen::Gap(last + 1..id)
        }
    }
}

#[cfg(test)]
mod tests;
