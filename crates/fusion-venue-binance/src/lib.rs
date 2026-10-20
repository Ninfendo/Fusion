//! Binance market data, turned into Fusion's market events.
//!
//! - [`parse_stream`], [`parse_snapshot`] and [`parse_trades`] read Binance's
//!   messages.
//! - [`Feed`] keeps each order book in sync with Binance's documented
//!   procedure, drops duplicates, and asks for what is missing.
//!
//! Everything here is pure; the connection that fetches and streams comes
//! on top.

mod feed;
mod symbols;
mod wire;

pub use feed::{Feed, MAX_BUFFERED, Note, Output, Want};
pub use symbols::{Endpoint, SymbolTable};
pub use wire::{Message, ParseError, parse_snapshot, parse_stream, parse_trades};
