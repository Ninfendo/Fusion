//! Two stream connections at once, around Binance's 24-hour limit.
//!
//! Both connections carry the same data, but either one can be ahead.
//! Data that fits on what the feed has passes, whichever connection brought
//! it. Data that comes too early is parked until the missing part arrives
//! from the other connection. Data the feed already has is dropped. So the
//! order in which the two connections deliver does not matter.
//!
//! The old connection may close once, for every instrument, the feed has
//! reached the first book update and the first trade the new connection
//! delivered: from there on the new connection has everything.

use std::collections::BTreeMap;

use fusion_domain::{InstrumentId, MarketEvent};
use tokio::time::Instant;

use crate::feed::Feed;
use crate::symbols::SymbolTable;

/// Parked events kept at most. Past it the oldest goes to the feed, which
/// then finds the gap and starts that book again.
pub(crate) const MAX_PARKED: usize = 10_000;

/// What to do with an event during an overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admit {
    /// Give it to the feed.
    Pass,
    /// Keep it until the missing part arrives.
    Park,
    /// The feed has it already, or it is from neither connection.
    Drop,
}

/// Whether numbers `first..=last` fit on a feed that is at `at`.
fn fit(first: u64, last: u64, at: u64) -> Admit {
    if last <= at {
        Admit::Drop
    } else if first <= at + 1 {
        Admit::Pass
    } else {
        Admit::Park
    }
}

/// How an event fits on the feed, or `None` if the feed has nothing to fit
/// it on yet (a book waiting for its snapshot, no trade yet).
fn fits(event: &MarketEvent, feed: &Feed) -> Option<Admit> {
    match event {
        MarketEvent::Delta(d) => feed
            .book(d.instrument)
            .map(|book| fit(d.first_update, d.last_update, book.update_id())),
        MarketEvent::Trade(t) => feed
            .last_trade(t.instrument)
            .map(|last| fit(t.id, t.id, last)),
        // Best prices and snapshots: the feed sorts them out.
        MarketEvent::Top(_) | MarketEvent::Snapshot(_) => Some(Admit::Pass),
    }
}

/// The order of parked events: by instrument, kind, then number.
fn order(event: &MarketEvent) -> (InstrumentId, u8, u64) {
    match event {
        MarketEvent::Delta(d) => (d.instrument, 0, d.first_update),
        MarketEvent::Trade(t) => (t.instrument, 1, t.id),
        MarketEvent::Top(t) => (t.instrument, 2, t.update_id),
        MarketEvent::Snapshot(s) => (s.instrument, 3, s.update_id),
    }
}

/// Two connections at once.
pub(crate) struct Overlap {
    pub(crate) old: u64,
    pub(crate) new: u64,
    pub(crate) started: Instant,
    first_update: BTreeMap<InstrumentId, u64>,
    first_trade: BTreeMap<InstrumentId, u64>,
    parked: Vec<MarketEvent>,
}

impl Overlap {
    pub(crate) fn new(old: u64, new: u64, started: Instant) -> Self {
        Self {
            old,
            new,
            started,
            first_update: BTreeMap::new(),
            first_trade: BTreeMap::new(),
            parked: Vec::new(),
        }
    }

    /// What to do with an event from connection `conn`.
    pub(crate) fn admit(&mut self, conn: u64, event: &MarketEvent, feed: &Feed) -> Admit {
        if conn != self.old && conn != self.new {
            return Admit::Drop;
        }
        if conn == self.new {
            self.note_first(event);
        }
        match fits(event, feed) {
            Some(admit) => admit,
            // Nothing to fit on yet: the old connection alone feeds it.
            None if conn == self.old => Admit::Pass,
            None => Admit::Drop,
        }
    }

    fn note_first(&mut self, event: &MarketEvent) {
        match event {
            MarketEvent::Delta(d) => {
                self.first_update
                    .entry(d.instrument)
                    .or_insert(d.first_update);
            }
            MarketEvent::Trade(t) => {
                self.first_trade.entry(t.instrument).or_insert(t.id);
            }
            MarketEvent::Top(_) | MarketEvent::Snapshot(_) => {}
        }
    }

    /// Parks an event. Returns the oldest parked event if too many wait:
    /// give it to the feed.
    pub(crate) fn park(&mut self, event: MarketEvent) -> Option<MarketEvent> {
        self.parked.push(event);
        (self.parked.len() > MAX_PARKED).then(|| self.parked.remove(0))
    }

    /// A parked event that fits now, if any. Parked events the feed already
    /// has are dropped on the way.
    pub(crate) fn unpark(&mut self, feed: &Feed) -> Option<MarketEvent> {
        self.parked
            .retain(|event| fits(event, feed) != Some(Admit::Drop));
        let at = self
            .parked
            .iter()
            .position(|event| fits(event, feed) == Some(Admit::Pass))?;
        Some(self.parked.remove(at))
    }

    /// Everything still parked, in order, for the feed when the overlap ends.
    pub(crate) fn drain(&mut self) -> Vec<MarketEvent> {
        let mut parked = std::mem::take(&mut self.parked);
        parked.sort_by_key(order);
        parked
    }

    /// True once, for every instrument, the feed has used the first book
    /// update and the first trade the new connection brought: from there
    /// on the new connection alone has everything.
    pub(crate) fn complete(&self, symbols: &SymbolTable, feed: &Feed) -> bool {
        symbols.ids().all(|id| {
            let books = self
                .first_update
                .get(&id)
                .is_some_and(|first| feed.book(id).is_some_and(|b| b.update_id() >= *first));
            let trades = self
                .first_trade
                .get(&id)
                .is_some_and(|first| feed.last_trade(id).is_some_and(|last| last >= *first));
            books && trades
        })
    }
}

#[cfg(test)]
mod tests;
