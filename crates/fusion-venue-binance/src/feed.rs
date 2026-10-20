//! Keeps the order books in sync and finds missed trades, following
//! Binance's documented procedure. No I/O: the feed says what it wants (a
//! book snapshot, missed trades), and the connection fetches it.
//!
//! Duplicates are expected: around the 24-hour limit two connections run
//! side by side, so the feed drops trades, deltas and best prices it has
//! already passed on.

use std::ops::Range;

use fusion_domain::{
    BookDelta, BookGap, BookSnapshot, DeltaOutcome, InstrumentId, MarketEvent, OrderBook, Seen,
    TopOfBook, Trade, TradeSequence,
};

use crate::symbols::SymbolTable;

/// Deltas kept while a snapshot is on its way. Far more than a snapshot
/// takes; past it the book starts again.
pub const MAX_BUFFERED: usize = 10_000;

/// Something the feed needs from the REST API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// A book snapshot.
    Snapshot(InstrumentId),
    /// Missed trades, from trade number `from` on.
    Trades {
        /// The instrument.
        instrument: InstrumentId,
        /// The first missed trade number.
        from: u64,
    },
}

/// Something worth logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Book updates were missed; the book starts again from a snapshot.
    BookGap(InstrumentId, BookGap),
    /// The snapshot was older than the buffered changes; asked again.
    SnapshotTooOld(InstrumentId),
    /// Too many changes waited for a snapshot; the book starts again.
    BufferFull(InstrumentId),
    /// Trades were missed; they are being fetched.
    MissedTrades(InstrumentId, Range<u64>),
    /// The REST API had none of the missed trades left; they are lost.
    TradesLost(InstrumentId, Vec<Range<u64>>),
}

/// What the feed did with one input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Market events to pass on, in order.
    pub events: Vec<MarketEvent>,
    /// What to fetch.
    pub wants: Vec<Want>,
    /// What to log.
    pub notes: Vec<Note>,
}

enum Book {
    /// Waiting for a snapshot, keeping the changes that arrive meanwhile.
    Waiting {
        buffer: Vec<BookDelta>,
        asked: bool,
    },
    Live(OrderBook),
}

#[derive(Default)]
struct Trades {
    sequence: TradeSequence,
    /// Trade numbers missed and not yet fetched.
    missing: Vec<Range<u64>>,
}

struct Instrument {
    id: InstrumentId,
    book: Book,
    trades: Trades,
    /// The update number of the last best prices passed on.
    top: u64,
}

/// The market data of every instrument in a [`SymbolTable`].
pub struct Feed {
    instruments: Vec<Instrument>,
}

impl Feed {
    /// A feed for every instrument in `symbols`; each book waits for its
    /// first changes and a snapshot.
    pub fn new(symbols: &SymbolTable) -> Self {
        Self {
            instruments: symbols
                .ids()
                .map(|id| Instrument {
                    id,
                    book: Book::Waiting {
                        buffer: Vec::new(),
                        asked: false,
                    },
                    trades: Trades::default(),
                    top: 0,
                })
                .collect(),
        }
    }

    fn slot(&mut self, id: InstrumentId) -> Option<&mut Instrument> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        self.instruments.get_mut(index)
    }

    /// The book of an instrument, once it is in sync.
    pub fn book(&self, id: InstrumentId) -> Option<&OrderBook> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        match &self.instruments.get(index)?.book {
            Book::Live(book) => Some(book),
            Book::Waiting { .. } => None,
        }
    }

    /// The number of the last trade passed on for an instrument.
    pub fn last_trade(&self, id: InstrumentId) -> Option<u64> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        self.instruments.get(index)?.trades.sequence.last()
    }

    /// Takes an event from the stream.
    pub fn on_event(&mut self, event: MarketEvent, out: &mut Output) {
        let Some(slot) = self.slot(event.instrument()) else {
            return;
        };
        match event {
            MarketEvent::Trade(trade) => slot.on_trade(trade, out),
            MarketEvent::Top(top) => slot.on_top(top, out),
            MarketEvent::Delta(delta) => slot.on_delta(delta, out),
            // Snapshots come from the REST API, through on_snapshot.
            MarketEvent::Snapshot(snapshot) => slot.on_snapshot(snapshot, out),
        }
    }

    /// Takes a book snapshot that a [`Want::Snapshot`] asked for.
    pub fn on_snapshot(&mut self, snapshot: BookSnapshot, out: &mut Output) {
        if let Some(slot) = self.slot(snapshot.instrument) {
            slot.on_snapshot(snapshot, out);
        }
    }

    /// Takes trades that a [`Want::Trades`] asked for, oldest first. Only
    /// missed ones are passed on; if some are still missing, they are
    /// asked for again.
    pub fn on_trades(&mut self, id: InstrumentId, trades: Vec<Trade>, out: &mut Output) {
        if let Some(slot) = self.slot(id) {
            slot.on_fetched_trades(trades, out);
        }
    }
}

impl Instrument {
    fn on_trade(&mut self, trade: Trade, out: &mut Output) {
        match self.trades.sequence.observe(trade.id) {
            Seen::Repeat => return,
            Seen::Gap(missed) => {
                out.notes.push(Note::MissedTrades(self.id, missed.clone()));
                out.wants.push(Want::Trades {
                    instrument: self.id,
                    from: missed.start,
                });
                self.trades.missing.push(missed);
            }
            Seen::First | Seen::Next => {}
        }
        out.events.push(MarketEvent::Trade(trade));
    }

    fn on_fetched_trades(&mut self, trades: Vec<Trade>, out: &mut Output) {
        let missing = &mut self.trades.missing;
        let before = out.events.len();
        for trade in trades {
            let Some(at) = missing.iter().position(|r| r.contains(&trade.id)) else {
                continue;
            };
            let range = missing.remove(at);
            // What is left of the range on each side of this trade.
            for part in [range.start..trade.id, trade.id + 1..range.end] {
                if !part.is_empty() {
                    missing.push(part);
                }
            }
            out.events.push(MarketEvent::Trade(trade));
        }
        missing.sort_by_key(|r| r.start);
        // A fetch that filled nothing will fill nothing next time either.
        if out.events.len() == before && !missing.is_empty() {
            out.notes
                .push(Note::TradesLost(self.id, std::mem::take(missing)));
        }
        if let Some(first) = missing.first() {
            out.wants.push(Want::Trades {
                instrument: self.id,
                from: first.start,
            });
        }
    }

    fn on_top(&mut self, top: TopOfBook, out: &mut Output) {
        if top.update_id > self.top {
            self.top = top.update_id;
            out.events.push(MarketEvent::Top(top));
        }
    }

    fn on_delta(&mut self, delta: BookDelta, out: &mut Output) {
        match &mut self.book {
            Book::Waiting { buffer, asked } => {
                if buffer.len() >= MAX_BUFFERED {
                    out.notes.push(Note::BufferFull(self.id));
                    buffer.clear();
                }
                if !*asked {
                    *asked = true;
                    out.wants.push(Want::Snapshot(self.id));
                }
                buffer.push(delta);
            }
            Book::Live(book) => match book.apply(&delta) {
                Ok(DeltaOutcome::Applied) => out.events.push(MarketEvent::Delta(delta)),
                Ok(DeltaOutcome::Stale) => {}
                Err(gap) => self.restart(gap, vec![delta], out),
            },
        }
    }

    /// The book missed updates: wait for a new snapshot, keeping `buffer`.
    fn restart(&mut self, gap: BookGap, buffer: Vec<BookDelta>, out: &mut Output) {
        out.notes.push(Note::BookGap(self.id, gap));
        out.wants.push(Want::Snapshot(self.id));
        self.book = Book::Waiting {
            buffer,
            asked: true,
        };
    }

    fn on_snapshot(&mut self, snapshot: BookSnapshot, out: &mut Output) {
        let Book::Waiting { buffer, .. } = &mut self.book else {
            return;
        };
        // Binance: a snapshot older than the first buffered change is no use.
        if buffer
            .first()
            .is_some_and(|first| snapshot.update_id < first.first_update)
        {
            out.notes.push(Note::SnapshotTooOld(self.id));
            out.wants.push(Want::Snapshot(self.id));
            return;
        }
        let buffered = std::mem::take(buffer);
        let mut book = OrderBook::from_snapshot(&snapshot);
        out.events.push(MarketEvent::Snapshot(snapshot));
        let mut rest = buffered.into_iter();
        while let Some(delta) = rest.next() {
            match book.apply(&delta) {
                Ok(DeltaOutcome::Applied) => out.events.push(MarketEvent::Delta(delta)),
                Ok(DeltaOutcome::Stale) => {}
                Err(gap) => {
                    let mut keep = vec![delta];
                    keep.extend(rest);
                    self.restart(gap, keep, out);
                    return;
                }
            }
        }
        self.book = Book::Live(book);
    }
}

#[cfg(test)]
mod tests;
