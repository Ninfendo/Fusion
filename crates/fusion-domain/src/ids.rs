//! Identifiers.
//!
//! Internal ids are small numbers. Ids that travel to and from a venue
//! (`ClOrdId`, `ExecId`) are strings, because the venue defines their form.

use std::fmt;

/// Fusion's own id for an instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentId(pub u32);

/// Fusion's own id for an order. It never changes, even when the order is
/// replaced and gets a new `ClOrdId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId(pub u64);

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Wraps the id as the venue or the client writes it.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The id as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// FIX ClOrdID (tag 11): the id of one request about an order. A new
    /// order, a cancel and a replace each use a new one.
    ClOrdId
);

string_id!(
    /// FIX ExecID (tag 17): the venue's id of one execution report.
    ExecId
);
