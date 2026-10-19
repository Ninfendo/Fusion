//! Time as the engine sees it.
//!
//! The engine never reads the system clock directly. It asks a [`Clock`], so a
//! replay or a backtest can feed it recorded time and get the same result.

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};

/// A point in time: nanoseconds since the Unix epoch, in UTC.
///
/// Stored as `i64` nanoseconds, which covers the years 1677 to 2262.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(i64);

impl Timestamp {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Self = Self(0);

    /// A timestamp from nanoseconds since the Unix epoch.
    pub const fn from_unix_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    /// A timestamp from milliseconds since the Unix epoch, or `None` on overflow.
    pub const fn from_unix_millis(millis: i64) -> Option<Self> {
        match millis.checked_mul(1_000_000) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// Nanoseconds since the Unix epoch.
    pub const fn unix_nanos(self) -> i64 {
        self.0
    }

    /// This timestamp plus `nanos`, or `None` on overflow.
    pub const fn checked_add_nanos(self, nanos: i64) -> Option<Self> {
        match self.0.checked_add(nanos) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }
}

/// The only source of time for the engine.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// A clock that only moves when told to, and never backwards.
///
/// Used by tests and by replay, where time comes from recorded events.
#[derive(Debug, Default)]
pub struct ManualClock {
    nanos: AtomicI64,
}

/// A [`ManualClock`] was asked to move backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWentBackwards {
    /// The clock's time when the request came.
    pub now: Timestamp,
    /// The earlier time that was asked for.
    pub requested: Timestamp,
}

impl fmt::Display for TimeWentBackwards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "clock cannot move backwards: now {} ns, requested {} ns",
            self.now.unix_nanos(),
            self.requested.unix_nanos()
        )
    }
}

impl std::error::Error for TimeWentBackwards {}

impl ManualClock {
    /// A clock that starts at `start`.
    pub fn new(start: Timestamp) -> Self {
        Self {
            nanos: AtomicI64::new(start.unix_nanos()),
        }
    }

    /// Moves the clock to `to`. Moving to the current time is allowed.
    pub fn advance_to(&self, to: Timestamp) -> Result<(), TimeWentBackwards> {
        self.nanos
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                (to.unix_nanos() >= now).then_some(to.unix_nanos())
            })
            .map(|_| ())
            .map_err(|now| TimeWentBackwards {
                now: Timestamp::from_unix_nanos(now),
                requested: to,
            })
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_nanos(self.nanos.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millis_convert_to_nanos() {
        let t = Timestamp::from_unix_millis(1_500).expect("no overflow");
        assert_eq!(t.unix_nanos(), 1_500_000_000);
    }

    #[test]
    fn millis_overflow_is_none() {
        assert_eq!(Timestamp::from_unix_millis(i64::MAX), None);
    }

    #[test]
    fn timestamps_order_by_time() {
        let early = Timestamp::from_unix_nanos(1);
        let late = Timestamp::from_unix_nanos(2);
        assert!(early < late);
        assert_eq!(early.checked_add_nanos(1), Some(late));
        assert_eq!(
            Timestamp::from_unix_nanos(i64::MAX).checked_add_nanos(1),
            None
        );
    }

    #[test]
    fn manual_clock_moves_forward() {
        let clock = ManualClock::new(Timestamp::from_unix_nanos(10));
        assert_eq!(clock.now(), Timestamp::from_unix_nanos(10));
        clock
            .advance_to(Timestamp::from_unix_nanos(25))
            .expect("forward is allowed");
        assert_eq!(clock.now(), Timestamp::from_unix_nanos(25));
    }

    #[test]
    fn manual_clock_allows_same_time() {
        let clock = ManualClock::new(Timestamp::from_unix_nanos(10));
        assert!(clock.advance_to(Timestamp::from_unix_nanos(10)).is_ok());
    }

    #[test]
    fn manual_clock_refuses_to_go_back() {
        let clock = ManualClock::new(Timestamp::from_unix_nanos(10));
        let err = clock
            .advance_to(Timestamp::from_unix_nanos(9))
            .expect_err("backwards is refused");
        assert_eq!(err.now, Timestamp::from_unix_nanos(10));
        assert_eq!(err.requested, Timestamp::from_unix_nanos(9));
        assert_eq!(clock.now(), Timestamp::from_unix_nanos(10));
    }
}
