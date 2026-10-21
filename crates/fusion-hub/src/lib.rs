//! The hub between Fusion's windows.
//!
//! Every window talks to one hub. The hub owns the layout and the link
//! router: a window never changes the layout itself, it sends a [`ToHub`]
//! message and applies the [`FromHub`] messages that come back. So one code
//! path serves every setup:
//!
//! - one window on its own, or in the browser: the hub runs in the window,
//!   through [`LocalBus`];
//! - one process per window: the hub runs in the launcher, and the messages
//!   travel over a local pipe as JSON lines ([`encode`], [`decode`]);
//! - browser tabs: the hub runs in the main tab ([`TabHub`]), and the
//!   other tabs post [`Envelope`]s over a BroadcastChannel.
//!
//! This crate does no I/O, so the hub is a pure state machine and every
//! rule is testable without processes or pipes. The launcher's decisions
//! are here too: which connection is which window ([`Switchboard`]) and
//! when to start a window process again ([`Supervisor`]).

mod hub;
mod local;
mod protocol;
mod supervisor;
mod switchboard;
mod tabs;

pub use hub::{Address, Hub, Output, Reply};
pub use local::{Bus, LocalBus};
pub use protocol::{Fdc3Context, FromHub, InstrumentIds, ToHub, decode, encode};
pub use supervisor::{AfterExit, CRASH_SPAN, Ended, MAX_CRASHES, Supervisor};
pub use switchboard::{ConnId, Switchboard};
pub use tabs::{
    Envelope, POPUP_WAIT_SECONDS, PendingPopups, Popup, TabHub, TabReply, query_for_window,
    window_from_query,
};
