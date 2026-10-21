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
//!
//! [`LiveFeed`] puts a link and a [`fusion_client_core::LiveMarket`]
//! together: the market data the app shows when a server is given.

mod feed;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use std::time::Duration;

use fusion_protocol::v1::ServerMessage;

pub use feed::LiveFeed;
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

/// The view server's address in a page's query, if any: `?server=ws://host:7070`,
/// or percent-encoded, `?window=2&server=ws%3A%2F%2Fhost%3A7070`.
pub fn server_from_query(query: &str) -> Option<String> {
    let value = query
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("server="))?;
    let url = percent_decode(value);
    (!url.is_empty()).then_some(url)
}

/// Turns each `%XX` into its byte. A `%` without two hex digits stays.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let hex = bytes
            .get(at + 1..at + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (byte, hex) {
            (b'%', Some(decoded)) => {
                out.push(decoded);
                at += 3;
            }
            _ => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_comes_from_the_query() {
        assert_eq!(
            server_from_query("?server=ws://127.0.0.1:7070"),
            Some("ws://127.0.0.1:7070".to_owned())
        );
        assert_eq!(
            server_from_query("?window=2&server=ws%3A%2F%2Fhost%3a7070"),
            Some("ws://host:7070".to_owned())
        );
        assert_eq!(server_from_query("?window=2"), None);
        assert_eq!(server_from_query(""), None);
        assert_eq!(server_from_query("?server="), None);
    }

    #[test]
    fn a_lone_percent_stays() {
        assert_eq!(percent_decode("a%"), "a%");
        assert_eq!(percent_decode("a%4"), "a%4");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
        assert_eq!(percent_decode("%+1"), "%+1");
    }

    #[test]
    fn waits_double_up_to_the_limit() {
        assert_eq!(retry_after(0), Duration::from_secs(1));
        assert_eq!(retry_after(1), Duration::from_secs(2));
        assert_eq!(retry_after(4), Duration::from_secs(16));
        assert_eq!(retry_after(5), MAX_RETRY);
        assert_eq!(retry_after(u32::MAX), MAX_RETRY);
    }
}
