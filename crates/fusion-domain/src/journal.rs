//! The event journal: an append-only log that the engine's state is rebuilt
//! from.
//!
//! Sequence numbers start at 1 and have no gaps. Time never goes backwards.
//! The PostgreSQL journal comes in a later milestone and implements the same
//! [`Journal`] trait.

use std::fmt;

use crate::time::Timestamp;

/// One journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<E> {
    /// The entry's position: 1, 2, 3 and so on.
    pub seq: u64,
    /// When the event happened, by the engine's clock.
    pub at: Timestamp,
    /// The event.
    pub event: E,
}

/// Why an append was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalError {
    /// The event is older than the last entry.
    TimeWentBackwards {
        /// Time of the last entry.
        last: Timestamp,
        /// Time of the refused event.
        at: Timestamp,
    },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeWentBackwards { last, at } => write!(
                f,
                "event at {} ns is older than the last entry at {} ns",
                at.unix_nanos(),
                last.unix_nanos()
            ),
        }
    }
}

impl std::error::Error for JournalError {}

/// An append-only event log.
pub trait Journal<E> {
    /// Why an operation failed.
    type Error: std::error::Error;

    /// Appends an event and returns its sequence number.
    fn append(&mut self, at: Timestamp, event: E) -> Result<u64, Self::Error>;

    /// Calls `visit` for every entry from sequence number `from` on, in order.
    fn replay(&self, from: u64, visit: &mut dyn FnMut(&Entry<E>)) -> Result<(), Self::Error>;

    /// The sequence number of the last entry, or 0 when the journal is empty.
    fn last_seq(&self) -> u64;
}

/// A journal in memory, for tests and backtests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryJournal<E> {
    entries: Vec<Entry<E>>,
}

impl<E> Default for MemoryJournal<E> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<E> MemoryJournal<E> {
    /// An empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// All entries, oldest first.
    pub fn entries(&self) -> &[Entry<E>] {
        &self.entries
    }
}

impl<E> Journal<E> for MemoryJournal<E> {
    type Error = JournalError;

    fn append(&mut self, at: Timestamp, event: E) -> Result<u64, JournalError> {
        if let Some(last) = self.entries.last()
            && at < last.at
        {
            return Err(JournalError::TimeWentBackwards { last: last.at, at });
        }
        let seq = self.last_seq() + 1;
        self.entries.push(Entry { seq, at, event });
        Ok(seq)
    }

    fn replay(&self, from: u64, visit: &mut dyn FnMut(&Entry<E>)) -> Result<(), JournalError> {
        let start = usize::try_from(from.saturating_sub(1)).unwrap_or(usize::MAX);
        self.entries.iter().skip(start).for_each(visit);
        Ok(())
    }

    fn last_seq(&self) -> u64 {
        self.entries.last().map_or(0, |entry| entry.seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(nanos: i64) -> Timestamp {
        Timestamp::from_unix_nanos(nanos)
    }

    fn collect(journal: &MemoryJournal<&'static str>, from: u64) -> Vec<(u64, &'static str)> {
        let mut seen = Vec::new();
        journal
            .replay(from, &mut |entry| seen.push((entry.seq, entry.event)))
            .expect("memory replay cannot fail");
        seen
    }

    #[test]
    fn sequence_numbers_start_at_one_without_gaps() {
        let mut journal = MemoryJournal::new();
        assert_eq!(journal.last_seq(), 0);
        assert_eq!(journal.append(t(1), "a"), Ok(1));
        assert_eq!(journal.append(t(2), "b"), Ok(2));
        assert_eq!(journal.append(t(2), "c"), Ok(3));
        assert_eq!(journal.last_seq(), 3);
        assert_eq!(journal.entries().len(), 3);
    }

    #[test]
    fn time_cannot_go_backwards() {
        let mut journal = MemoryJournal::new();
        journal.append(t(5), "a").expect("first append");
        assert_eq!(
            journal.append(t(4), "b"),
            Err(JournalError::TimeWentBackwards {
                last: t(5),
                at: t(4)
            })
        );
        assert_eq!(journal.last_seq(), 1);
    }

    #[test]
    fn replay_starts_at_the_given_sequence_number() {
        let mut journal = MemoryJournal::new();
        for (i, event) in ["a", "b", "c"].into_iter().enumerate() {
            journal.append(t(i as i64), event).expect("append");
        }
        assert_eq!(collect(&journal, 0), [(1, "a"), (2, "b"), (3, "c")]);
        assert_eq!(collect(&journal, 1), [(1, "a"), (2, "b"), (3, "c")]);
        assert_eq!(collect(&journal, 3), [(3, "c")]);
        assert!(collect(&journal, 4).is_empty());
    }

    #[test]
    fn the_error_reads_as_a_sentence() {
        let err = JournalError::TimeWentBackwards {
            last: t(5),
            at: t(4),
        };
        assert_eq!(
            err.to_string(),
            "event at 4 ns is older than the last entry at 5 ns"
        );
    }
}
