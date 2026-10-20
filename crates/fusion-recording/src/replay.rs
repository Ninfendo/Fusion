//! Replaying a recording: rebuild every book, check it, find missing trades.

use std::collections::BTreeMap;
use std::ops::Range;

use fusion_domain::{InstrumentId, MarketEvent, OrderBook, Timestamp};

use crate::{BookCheck, Record, Row};

/// What a replay found for one instrument.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolReport {
    /// Trades seen.
    pub trades: u64,
    /// Best-price rows seen.
    pub tops: u64,
    /// Book snapshots seen. More than one means the book started again.
    pub snapshots: u64,
    /// Book changes applied.
    pub deltas: u64,
    /// Book changes that did not fit the book: a hole in the recording.
    pub book_gaps: u64,
    /// Checks where the rebuilt book had the recorded fingerprint.
    pub checks_passed: u64,
    /// Checks where it did not, by update number.
    pub checks_failed: Vec<u64>,
    /// Trade numbers missing between the first and the last trade.
    pub trade_gaps: Vec<Range<u64>>,
}

impl SymbolReport {
    /// True if nothing is missing and every check passed.
    pub fn is_clean(&self) -> bool {
        self.book_gaps == 0 && self.checks_failed.is_empty() && self.trade_gaps.is_empty()
    }
}

/// What a replay found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Rows replayed.
    pub rows: u64,
    /// The first and the last receive time.
    pub span: Option<(Timestamp, Timestamp)>,
    /// The longest time between two rows, in nanoseconds, and when it
    /// started. A recorder that was down shows here.
    pub longest_silence: Option<(Timestamp, i64)>,
    /// Per instrument.
    pub instruments: BTreeMap<InstrumentId, SymbolReport>,
}

impl ReplayReport {
    /// True if every instrument is clean.
    pub fn is_clean(&self) -> bool {
        self.instruments.values().all(SymbolReport::is_clean)
    }
}

/// Rebuilds the books of a recording, row by row.
#[derive(Debug, Default)]
pub struct Replay {
    books: BTreeMap<InstrumentId, OrderBook>,
    trade_ids: BTreeMap<InstrumentId, Vec<u64>>,
    report: ReplayReport,
}

impl Replay {
    /// The book of an instrument as rebuilt so far.
    pub fn book(&self, id: InstrumentId) -> Option<&OrderBook> {
        self.books.get(&id)
    }

    /// Takes the next row.
    pub fn apply(&mut self, row: &Row) {
        self.report.rows += 1;
        if let Some((_, last)) = self.report.span {
            let silence = row.received.unix_nanos().saturating_sub(last.unix_nanos());
            let longest = self.report.longest_silence.map_or(0, |(_, n)| n);
            if silence > longest {
                self.report.longest_silence = Some((last, silence));
            }
        }
        self.report.span = Some(match self.report.span {
            None => (row.received, row.received),
            Some((first, last)) => (first.min(row.received), last.max(row.received)),
        });
        let id = row.record.instrument();
        let stats = self.report.instruments.entry(id).or_default();
        match &row.record {
            Record::Event(MarketEvent::Trade(trade)) => {
                stats.trades += 1;
                self.trade_ids.entry(id).or_default().push(trade.id);
            }
            Record::Event(MarketEvent::Top(_)) => stats.tops += 1,
            Record::Event(MarketEvent::Snapshot(snapshot)) => {
                stats.snapshots += 1;
                self.books.insert(id, OrderBook::from_snapshot(snapshot));
            }
            Record::Event(MarketEvent::Delta(delta)) => {
                stats.deltas += 1;
                let applied = self.books.get_mut(&id).map(|book| book.apply(delta));
                if !matches!(applied, Some(Ok(_))) {
                    stats.book_gaps += 1;
                }
            }
            Record::Check(check) => check_book(self.books.get(&id), check, stats),
        }
    }

    /// The report, with the trade gaps worked out.
    pub fn finish(mut self) -> ReplayReport {
        for (id, mut ids) in self.trade_ids {
            ids.sort_unstable();
            ids.dedup();
            let gaps = ids
                .windows(2)
                .filter(|pair| pair[1] > pair[0] + 1)
                .map(|pair| pair[0] + 1..pair[1])
                .collect();
            self.report.instruments.entry(id).or_default().trade_gaps = gaps;
        }
        self.report
    }
}

fn check_book(book: Option<&OrderBook>, check: &BookCheck, stats: &mut SymbolReport) {
    let same = book.is_some_and(|b| b.update_id() == check.update_id && b.digest() == check.digest);
    if same {
        stats.checks_passed += 1;
    } else {
        stats.checks_failed.push(check.update_id);
    }
}

#[cfg(test)]
mod tests {
    use fusion_domain::{BookSnapshot, InstrumentId};

    use super::*;

    #[test]
    fn each_problem_alone_makes_a_report_unclean() {
        let clean = SymbolReport::default();
        assert!(clean.is_clean());
        let hole = SymbolReport {
            book_gaps: 1,
            ..SymbolReport::default()
        };
        let failed = SymbolReport {
            checks_failed: vec![7],
            ..SymbolReport::default()
        };
        let missing = SymbolReport {
            trade_gaps: std::iter::once(3..4).collect(),
            ..SymbolReport::default()
        };
        for report in [hole, failed, missing] {
            assert!(!report.is_clean(), "{report:?}");
        }
    }

    #[test]
    fn a_check_needs_the_same_number_and_the_same_fingerprint() {
        let book = OrderBook::from_snapshot(&BookSnapshot {
            instrument: InstrumentId(1),
            update_id: 7,
            bids: vec![],
            asks: vec![],
        });
        let check = |update_id, digest| BookCheck {
            instrument: InstrumentId(1),
            update_id,
            digest,
        };
        let mut stats = SymbolReport::default();
        check_book(Some(&book), &check(7, book.digest()), &mut stats);
        check_book(Some(&book), &check(7, book.digest() ^ 1), &mut stats);
        check_book(Some(&book), &check(8, book.digest()), &mut stats);
        check_book(None, &check(7, book.digest()), &mut stats);
        assert_eq!(stats.checks_passed, 1);
        assert_eq!(stats.checks_failed, [7, 8, 7]);
    }
}
