//! A made-up market, for load tests and for working offline.
//!
//! [`SyntheticMarket`] makes market events for any number of instruments.
//! Each book has [`DEPTH`] levels a side, one tick apart, with a one-tick
//! spread. A step changes one instrument: a trade at the bid or the ask, a
//! new quantity at a few levels, or a move of the whole book by one tick.
//! Every book change is a delta that follows the one before, so a client
//! keeps the same book as the market. It is seeded: the same seed gives the
//! same events. It does no I/O and keeps no clock; the caller says when.

use std::collections::VecDeque;

use fusion_domain::{
    BookDelta, BookSnapshot, Decimal, InstrumentId, Level, MarketEvent, Price, Quantity, Side,
    Timestamp, TopOfBook, Trade,
};

/// Levels on each side of every book.
pub const DEPTH: usize = 20;

/// Prices move in cents.
const PRICE_SCALE: u32 = 2;
/// Quantities are in thousandths.
const QUANTITY_SCALE: u32 = 3;

/// SplitMix64: a small, fast, seeded random generator. Fine for made-up
/// data; never for anything secret.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `n`, which must not be 0.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// An index below `n`, which must not be 0.
    fn index(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        usize::try_from(self.below(n)).unwrap_or(0)
    }

    /// A quantity of 1 to 1000 thousandths, as a whole number.
    fn lots(&mut self) -> i64 {
        i64::try_from(self.below(1000)).unwrap_or(0) + 1
    }

    fn coin(&mut self) -> bool {
        self.below(2) == 0
    }
}

fn price(ticks: i64) -> Price {
    Price::new(Decimal::new(ticks, PRICE_SCALE))
}

fn quantity(lots: i64) -> Quantity {
    Quantity::new(Decimal::new(lots, QUANTITY_SCALE))
}

fn level(ticks: i64, lots: i64) -> Level {
    Level::new(price(ticks), quantity(lots))
}

/// How many ticks the `i`-th level is from the best price.
fn ticks_away(i: usize) -> i64 {
    i64::try_from(i).unwrap_or(i64::MAX)
}

/// One instrument's book: `bids[i]` is `i` ticks below the best bid, and
/// `asks[i]` is `i` ticks above the best ask, which is one tick above the
/// best bid.
#[derive(Debug, Clone)]
struct Book {
    id: InstrumentId,
    best_bid: i64,
    bids: VecDeque<i64>,
    asks: VecDeque<i64>,
    update_id: u64,
}

impl Book {
    fn best_ask(&self) -> i64 {
        self.best_bid + 1
    }

    fn bid_levels(&self) -> Vec<Level> {
        (self.bids.iter().enumerate())
            .map(|(i, lots)| level(self.best_bid - ticks_away(i), *lots))
            .collect()
    }

    fn ask_levels(&self) -> Vec<Level> {
        (self.asks.iter().enumerate())
            .map(|(i, lots)| level(self.best_ask() + ticks_away(i), *lots))
            .collect()
    }

    fn snapshot(&self) -> BookSnapshot {
        BookSnapshot {
            instrument: self.id,
            update_id: self.update_id,
            bids: self.bid_levels(),
            asks: self.ask_levels(),
        }
    }

    fn top(&self) -> TopOfBook {
        TopOfBook {
            instrument: self.id,
            update_id: self.update_id,
            bid: level(self.best_bid, self.bids[0]),
            ask: level(self.best_ask(), self.asks[0]),
        }
    }

    fn delta(&mut self, time: Timestamp, bids: Vec<Level>, asks: Vec<Level>) -> BookDelta {
        self.update_id += 1;
        BookDelta {
            instrument: self.id,
            first_update: self.update_id,
            last_update: self.update_id,
            time,
            bids,
            asks,
        }
    }

    /// New quantities at one to three levels near the top of one side.
    fn requote(&mut self, rng: &mut Rng, time: Timestamp) -> BookDelta {
        let buy = rng.coin();
        let mut changed = Vec::new();
        for _ in 0..=rng.below(3) {
            let i = rng.index(DEPTH / 2);
            let lots = rng.lots();
            let ticks = if buy {
                self.bids[i] = lots;
                self.best_bid - ticks_away(i)
            } else {
                self.asks[i] = lots;
                self.best_ask() + ticks_away(i)
            };
            changed.push(level(ticks, lots));
        }
        if buy {
            self.delta(time, changed, Vec::new())
        } else {
            self.delta(time, Vec::new(), changed)
        }
    }

    /// The whole book one tick up: the best ask is taken and becomes the
    /// best bid. A new ask joins at the far end and the farthest bid
    /// leaves, so each side keeps [`DEPTH`] levels.
    fn up(&mut self, rng: &mut Rng, time: Timestamp) -> BookDelta {
        let far = ticks_away(DEPTH);
        let (new_lots, far_lots) = (rng.lots(), rng.lots());
        let taken = self.best_ask();
        let gone_bid = self.best_bid - far + 1;
        self.asks.pop_front();
        self.asks.push_back(far_lots);
        self.bids.pop_back();
        self.bids.push_front(new_lots);
        self.best_bid = taken;
        let new_ask = self.best_ask() + far - 1;
        let bids = vec![level(taken, new_lots), level(gone_bid, 0)];
        let asks = vec![level(taken, 0), level(new_ask, far_lots)];
        self.delta(time, bids, asks)
    }

    /// The whole book one tick down, the mirror of [`Book::up`].
    fn down(&mut self, rng: &mut Rng, time: Timestamp) -> BookDelta {
        let far = ticks_away(DEPTH);
        let (new_lots, far_lots) = (rng.lots(), rng.lots());
        let taken = self.best_bid;
        let gone_ask = self.best_ask() + far - 1;
        self.bids.pop_front();
        self.bids.push_back(far_lots);
        self.asks.pop_back();
        self.asks.push_front(new_lots);
        self.best_bid = taken - 1;
        let new_bid = self.best_bid - far + 1;
        let bids = vec![level(taken, 0), level(new_bid, far_lots)];
        let asks = vec![level(taken, new_lots), level(gone_ask, 0)];
        self.delta(time, bids, asks)
    }

    /// One tick up or down; always up near zero, so prices stay positive.
    fn shift(&mut self, rng: &mut Rng, time: Timestamp) -> BookDelta {
        if self.best_bid <= ticks_away(DEPTH) + 1 || rng.coin() {
            self.up(rng, time)
        } else {
            self.down(rng, time)
        }
    }

    fn trade(&self, rng: &mut Rng, id: u64, time: Timestamp) -> Trade {
        let (taker, ticks) = if rng.coin() {
            (Side::Buy, self.best_ask())
        } else {
            (Side::Sell, self.best_bid)
        };
        Trade {
            instrument: self.id,
            id,
            price: price(ticks),
            quantity: quantity(rng.lots()),
            taker,
            time,
        }
    }
}

/// A made-up market of many instruments.
#[derive(Debug, Clone)]
pub struct SyntheticMarket {
    rng: Rng,
    books: Vec<Book>,
    next_trade: u64,
}

impl SyntheticMarket {
    /// A market of `count` instruments, numbered from 1 and named
    /// `SYN0001`, `SYN0002`, and so on.
    pub fn new(count: usize, seed: u64) -> Self {
        let mut rng = Rng(seed);
        let books = (1..=count)
            .map(|n| Book {
                id: InstrumentId(u32::try_from(n).unwrap_or(u32::MAX)),
                // Between 10.00 and 1,000.00.
                best_bid: 1_000 + i64::try_from(rng.below(99_000)).unwrap_or(0),
                bids: (0..DEPTH).map(|_| rng.lots()).collect(),
                asks: (0..DEPTH).map(|_| rng.lots()).collect(),
                update_id: 1,
            })
            .collect();
        Self {
            rng,
            books,
            next_trade: 1,
        }
    }

    /// The symbols, in instrument order.
    pub fn symbols(&self) -> Vec<String> {
        (1..=self.books.len())
            .map(|n| format!("SYN{n:04}"))
            .collect()
    }

    /// Every book as it is now, with its best prices: what a client needs
    /// first.
    pub fn snapshots(&self) -> Vec<MarketEvent> {
        self.books
            .iter()
            .flat_map(|book| {
                [
                    MarketEvent::Snapshot(book.snapshot()),
                    MarketEvent::Top(book.top()),
                ]
            })
            .collect()
    }

    /// One change on one instrument, at `time`: a trade (3 in 10), new
    /// quantities (5 in 10), or a move of the book (2 in 10). A book change
    /// comes with the new best prices.
    pub fn step(&mut self, time: Timestamp) -> Vec<MarketEvent> {
        if self.books.is_empty() {
            return Vec::new();
        }
        let n = self.rng.index(self.books.len());
        let roll = self.rng.below(10);
        let book = &mut self.books[n];
        if roll < 3 {
            let trade = book.trade(&mut self.rng, self.next_trade, time);
            self.next_trade += 1;
            return vec![MarketEvent::Trade(trade)];
        }
        let delta = if roll < 8 {
            book.requote(&mut self.rng, time)
        } else {
            book.shift(&mut self.rng, time)
        };
        vec![MarketEvent::Delta(delta), MarketEvent::Top(book.top())]
    }
}

#[cfg(test)]
mod tests;
