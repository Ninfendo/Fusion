//! FIX code values Fusion uses in its own model, with their FIX 4.4 codes.
//!
//! The order model speaks FIX for every venue, also for venues that do not use
//! FIX on the wire. Their adapters translate into these values.

use std::fmt;

/// FIX Side (tag 54).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    /// `1`
    Buy,
    /// `2`
    Sell,
}

impl Side {
    /// Both sides.
    pub const ALL: [Self; 2] = [Self::Buy, Self::Sell];

    /// The FIX code.
    pub const fn as_fix(self) -> char {
        match self {
            Self::Buy => '1',
            Self::Sell => '2',
        }
    }

    /// The side for a FIX code, or `None` for a code Fusion does not support.
    pub const fn from_fix(code: char) -> Option<Self> {
        match code {
            '1' => Some(Self::Buy),
            '2' => Some(Self::Sell),
            _ => None,
        }
    }
}

/// FIX OrdStatus (tag 39): the state of an order.
///
/// FIX 4.4 retired code `5` (Replaced); a replace is reported with ExecType
/// Replaced and the order's real state in OrdStatus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrdStatus {
    /// `0`
    New,
    /// `1`
    PartiallyFilled,
    /// `2`
    Filled,
    /// `3`
    DoneForDay,
    /// `4`
    Canceled,
    /// `6`
    PendingCancel,
    /// `7`
    Stopped,
    /// `8`
    Rejected,
    /// `9`
    Suspended,
    /// `A`
    PendingNew,
    /// `B`
    Calculated,
    /// `C`
    Expired,
    /// `D`
    AcceptedForBidding,
    /// `E`
    PendingReplace,
}

impl OrdStatus {
    /// Every status, for exhaustive tests.
    pub const ALL: [Self; 14] = [
        Self::New,
        Self::PartiallyFilled,
        Self::Filled,
        Self::DoneForDay,
        Self::Canceled,
        Self::PendingCancel,
        Self::Stopped,
        Self::Rejected,
        Self::Suspended,
        Self::PendingNew,
        Self::Calculated,
        Self::Expired,
        Self::AcceptedForBidding,
        Self::PendingReplace,
    ];

    /// The FIX code.
    pub const fn as_fix(self) -> char {
        match self {
            Self::New => '0',
            Self::PartiallyFilled => '1',
            Self::Filled => '2',
            Self::DoneForDay => '3',
            Self::Canceled => '4',
            Self::PendingCancel => '6',
            Self::Stopped => '7',
            Self::Rejected => '8',
            Self::Suspended => '9',
            Self::PendingNew => 'A',
            Self::Calculated => 'B',
            Self::Expired => 'C',
            Self::AcceptedForBidding => 'D',
            Self::PendingReplace => 'E',
        }
    }

    /// The status for a FIX code, or `None` for an unknown or retired code.
    pub const fn from_fix(code: char) -> Option<Self> {
        Some(match code {
            '0' => Self::New,
            '1' => Self::PartiallyFilled,
            '2' => Self::Filled,
            '3' => Self::DoneForDay,
            '4' => Self::Canceled,
            '6' => Self::PendingCancel,
            '7' => Self::Stopped,
            '8' => Self::Rejected,
            '9' => Self::Suspended,
            'A' => Self::PendingNew,
            'B' => Self::Calculated,
            'C' => Self::Expired,
            'D' => Self::AcceptedForBidding,
            'E' => Self::PendingReplace,
            _ => return None,
        })
    }

    /// True if the order can never trade again: Filled, Canceled, Rejected
    /// or Expired.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired
        )
    }

    /// True if the venue still holds the unfilled rest of the order, so
    /// LeavesQty must equal OrderQty minus CumQty.
    pub const fn holds_leaves(self) -> bool {
        matches!(
            self,
            Self::PendingNew
                | Self::New
                | Self::PartiallyFilled
                | Self::PendingCancel
                | Self::PendingReplace
                | Self::Stopped
                | Self::Suspended
                | Self::AcceptedForBidding
        )
    }

    /// True for a resting order with no request in flight: PendingNew, New
    /// or PartiallyFilled.
    pub const fn is_working(self) -> bool {
        matches!(self, Self::PendingNew | Self::New | Self::PartiallyFilled)
    }
}

impl fmt::Display for OrdStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// FIX ExecType (tag 150): what happened, in one execution report.
///
/// FIX 4.2 codes `1` (Partial fill) and `2` (Fill) are read as [`ExecType::Trade`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecType {
    /// `0`
    New,
    /// `3`
    DoneForDay,
    /// `4`
    Canceled,
    /// `5`
    Replaced,
    /// `6`
    PendingCancel,
    /// `7`
    Stopped,
    /// `8`
    Rejected,
    /// `9`
    Suspended,
    /// `A`
    PendingNew,
    /// `B`
    Calculated,
    /// `C`
    Expired,
    /// `D`
    Restated,
    /// `E`
    PendingReplace,
    /// `F`
    Trade,
    /// `G`
    TradeCorrect,
    /// `H`
    TradeCancel,
    /// `I`
    OrderStatus,
}

impl ExecType {
    /// Every exec type, for exhaustive tests.
    pub const ALL: [Self; 17] = [
        Self::New,
        Self::DoneForDay,
        Self::Canceled,
        Self::Replaced,
        Self::PendingCancel,
        Self::Stopped,
        Self::Rejected,
        Self::Suspended,
        Self::PendingNew,
        Self::Calculated,
        Self::Expired,
        Self::Restated,
        Self::PendingReplace,
        Self::Trade,
        Self::TradeCorrect,
        Self::TradeCancel,
        Self::OrderStatus,
    ];

    /// The FIX 4.4 code.
    pub const fn as_fix(self) -> char {
        match self {
            Self::New => '0',
            Self::DoneForDay => '3',
            Self::Canceled => '4',
            Self::Replaced => '5',
            Self::PendingCancel => '6',
            Self::Stopped => '7',
            Self::Rejected => '8',
            Self::Suspended => '9',
            Self::PendingNew => 'A',
            Self::Calculated => 'B',
            Self::Expired => 'C',
            Self::Restated => 'D',
            Self::PendingReplace => 'E',
            Self::Trade => 'F',
            Self::TradeCorrect => 'G',
            Self::TradeCancel => 'H',
            Self::OrderStatus => 'I',
        }
    }

    /// The exec type for a FIX code, or `None` for an unknown code.
    pub const fn from_fix(code: char) -> Option<Self> {
        Some(match code {
            '0' => Self::New,
            '1' | '2' | 'F' => Self::Trade,
            '3' => Self::DoneForDay,
            '4' => Self::Canceled,
            '5' => Self::Replaced,
            '6' => Self::PendingCancel,
            '7' => Self::Stopped,
            '8' => Self::Rejected,
            '9' => Self::Suspended,
            'A' => Self::PendingNew,
            'B' => Self::Calculated,
            'C' => Self::Expired,
            'D' => Self::Restated,
            'E' => Self::PendingReplace,
            'G' => Self::TradeCorrect,
            'H' => Self::TradeCancel,
            'I' => Self::OrderStatus,
            _ => return None,
        })
    }

    /// The order statuses a venue may report together with this exec type.
    pub const fn allows_status(self, status: OrdStatus) -> bool {
        use OrdStatus as S;
        match self {
            Self::PendingNew => matches!(status, S::PendingNew),
            Self::New => matches!(status, S::New | S::PendingCancel | S::PendingReplace),
            Self::Rejected => matches!(status, S::Rejected),
            Self::Trade => matches!(
                status,
                S::PartiallyFilled
                    | S::Filled
                    | S::PendingCancel
                    | S::PendingReplace
                    | S::Canceled
                    | S::Expired
                    | S::DoneForDay
                    | S::Stopped
            ),
            Self::Canceled => matches!(status, S::Canceled),
            Self::Expired => matches!(status, S::Expired),
            Self::Replaced => matches!(
                status,
                S::New | S::PartiallyFilled | S::Filled | S::PendingCancel | S::PendingReplace
            ),
            Self::PendingCancel => matches!(status, S::PendingCancel),
            Self::PendingReplace => matches!(status, S::PendingReplace),
            Self::DoneForDay => matches!(status, S::DoneForDay),
            Self::Stopped => matches!(status, S::Stopped),
            Self::Suspended => matches!(status, S::Suspended),
            Self::Calculated => matches!(status, S::Calculated),
            Self::Restated | Self::TradeCorrect | Self::TradeCancel | Self::OrderStatus => true,
        }
    }
}

impl fmt::Display for ExecType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_codes_round_trip() {
        for side in Side::ALL {
            assert_eq!(Side::from_fix(side.as_fix()), Some(side));
        }
        assert_eq!(Side::from_fix('3'), None);
    }

    #[test]
    fn ord_status_codes_round_trip() {
        for status in OrdStatus::ALL {
            assert_eq!(OrdStatus::from_fix(status.as_fix()), Some(status));
        }
    }

    #[test]
    fn retired_and_unknown_ord_status_codes_are_none() {
        assert_eq!(OrdStatus::from_fix('5'), None);
        assert_eq!(OrdStatus::from_fix('Z'), None);
    }

    #[test]
    fn exec_type_codes_round_trip() {
        for exec_type in ExecType::ALL {
            assert_eq!(ExecType::from_fix(exec_type.as_fix()), Some(exec_type));
        }
        assert_eq!(ExecType::from_fix('Z'), None);
    }

    #[test]
    fn fix_42_fill_codes_read_as_trade() {
        assert_eq!(ExecType::from_fix('1'), Some(ExecType::Trade));
        assert_eq!(ExecType::from_fix('2'), Some(ExecType::Trade));
    }

    #[test]
    fn terminal_statuses_hold_no_leaves() {
        for status in OrdStatus::ALL {
            if status.is_terminal() {
                assert!(
                    !status.holds_leaves(),
                    "{status} is terminal but holds leaves"
                );
            }
        }
    }

    #[test]
    fn every_exec_type_allows_at_least_one_status() {
        for exec_type in ExecType::ALL {
            assert!(
                OrdStatus::ALL.iter().any(|s| exec_type.allows_status(*s)),
                "{exec_type} allows no status"
            );
        }
    }

    #[test]
    fn a_trade_never_reports_an_unfilled_status() {
        for status in [OrdStatus::New, OrdStatus::PendingNew, OrdStatus::Rejected] {
            assert!(!ExecType::Trade.allows_status(status));
        }
    }
}
