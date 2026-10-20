//! The view server's engine.
//!
//! The server keeps every table here. A client opens [`Viewport`]s onto
//! them: a window with columns, a sort, a CEL filter and a row range. Each
//! [`Viewport::poll`] gives the message to send: a snapshot first, then only
//! what changed in range, or nothing. The server polls each viewport at its
//! rate (20 times a second by default), so a price that ticks a hundred
//! times a second costs a client at most twenty small messages.
//!
//! A [`Session`] holds one client's viewports and polls each at its rate.
//! [`MarketTables`] turns market events into the `prices` table and one
//! `book/<SYMBOL>` table per instrument.
//!
//! A pass filters and sorts the whole table. That is fine for the tables of
//! milestone 4 (hundreds of rows); a table of a hundred thousand rows will
//! need an index that keeps the order between passes.

mod filter;
mod market;
mod session;
mod table;
mod viewport;

pub use filter::Filter;
pub use market::{
    BOOK_COLUMNS, BOOK_DEPTH, MarketTables, PRICE_COLUMNS, PRICES, book_key, book_table,
};
pub use session::{LayoutRequest, Session, Tables, ack, error};
pub use table::Table;
pub use viewport::{DEFAULT_RATE_HZ, MAX_ROWS, Viewport};

/// Why a viewport could not open.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// The table does not exist.
    #[error("there is no table {0}")]
    UnknownTable(String),
    /// A column in the spec does not exist.
    #[error("there is no column {0}")]
    UnknownColumn(String),
    /// The filter does not compile.
    #[error("the filter does not read: {0}")]
    Filter(String),
}
