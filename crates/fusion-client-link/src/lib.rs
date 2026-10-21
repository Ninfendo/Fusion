//! The client's line to the view server.
//!
//! [`Link`] is a WebSocket that reconnects by itself, after a wait
//! that doubles from 1 s up to 30 s. Its events say when it connected, so
//! the client opens its viewports again ([`LinkEvent::Connected`]). A
//! message sent while it is not connected is dropped: everything is opened
//! again on the next connection anyway.
//!
//! On desktop it is a thread with its own WebSocket; in the browser it is
//! the browser's WebSocket. Both call `wake` after each event, so a window
//! draws at once instead of on its next timer.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use std::time::Duration;

use fusion_protocol::v1::ServerMessage;

#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeLink as Link;
#[cfg(target_arch = "wasm32")]
pub use web::WebLink as Link;

/// What happened on the link.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// Connected: open the viewports.
    Connected,
    /// A message from the server.
    Message(ServerMessage),
    /// The connection ended, and why; it is tried again.
    Disconnected(String),
}

/// The first wait before connecting again.
pub const FIRST_RETRY: Duration = Duration::from_secs(1);

/// The longest wait before connecting again.
pub const MAX_RETRY: Duration = Duration::from_secs(30);

/// The wait after `failures` failed tries in a row.
pub fn retry_after(failures: u32) -> Duration {
    FIRST_RETRY
        .saturating_mul(2u32.saturating_pow(failures.min(16)))
        .min(MAX_RETRY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_up_to_the_limit() {
        assert_eq!(retry_after(0), Duration::from_secs(1));
        assert_eq!(retry_after(1), Duration::from_secs(2));
        assert_eq!(retry_after(4), Duration::from_secs(16));
        assert_eq!(retry_after(5), MAX_RETRY);
        assert_eq!(retry_after(u32::MAX), MAX_RETRY);
    }
}
