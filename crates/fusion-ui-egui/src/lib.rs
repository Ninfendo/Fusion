//! Fusion's egui front end: the panel views, the panel registry and the app.
//!
//! The same [`FusionApp`] runs as the desktop program (`fusion-window`) and,
//! later, in the browser. All panel logic lives in `fusion-client-core`; the
//! views here only draw it.

mod app;
pub mod panels;

pub use app::{FusionApp, LAYOUT_KEY, THEME_KEY, default_layout};
