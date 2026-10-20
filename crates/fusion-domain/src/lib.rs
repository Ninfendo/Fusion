//! Core domain types for Fusion.
//!
//! This crate has no I/O and no async runtime. It must compile to
//! `wasm32-unknown-unknown`, because the browser client uses the same types.

pub mod fix;
pub mod ids;
pub mod instrument;
pub mod journal;
pub mod market;
pub mod num;
pub mod order;
pub mod time;

pub use fix::{ExecType, OrdStatus, Side};
pub use ids::{ClOrdId, ExecId, InstrumentId, OrderId};
pub use instrument::{Instrument, InstrumentError};
pub use journal::{Entry, Journal, JournalError, MemoryJournal};
pub use market::{
    BookDelta, BookGap, BookSnapshot, DeltaOutcome, Level, OrderBook, Seen, TopOfBook, Trade,
    TradeSequence,
};
pub use num::{Decimal, Price, Quantity, notional};
pub use order::{
    Applied, ExecutionReport, Fill, NewOrder, Order, OrderError, OrderEvent, PendingRequest,
    QuantityField, RequestKind,
};
pub use time::{Clock, ManualClock, TimeWentBackwards, Timestamp};
