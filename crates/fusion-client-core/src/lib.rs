//! Fusion's client logic.
//!
//! This crate holds the presentation models: plain structs with all the state
//! a panel shows, and an `update` that applies a user action and returns the
//! effects to carry out. It has no UI library and no async runtime, so every
//! panel is testable without a screen, and a second UI library could reuse it.
//!
//! The panels read market data through [`MarketData`]. The app keeps a
//! [`MarketFeed`] up to date: [`demo::DemoFeed`] with made-up prices, or a
//! live feed from the view server, built on [`LiveMarket`].

pub mod demo;
pub mod ladder;
pub mod links;
pub mod live_market;
pub mod market;
pub mod viewport;
pub mod watchlist;
pub mod workspace;

pub use fusion_domain::{Decimal, Price, Quantity};
pub use live_market::LiveMarket;
pub use market::{Book, FeedStatus, Level, MarketData, MarketFeed, Quote};
pub use viewport::{Missed, ViewportCache};
