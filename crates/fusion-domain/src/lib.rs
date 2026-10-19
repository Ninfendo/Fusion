//! Core domain types for Fusion.
//!
//! This crate has no I/O and no async runtime. It must compile to
//! `wasm32-unknown-unknown`, because the browser client uses the same types.

pub mod time;

pub use time::{Clock, ManualClock, TimeWentBackwards, Timestamp};
