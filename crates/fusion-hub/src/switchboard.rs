//! Which connection is which window, for a hub that talks to window
//! processes over pipes.

use std::collections::BTreeMap;

use fusion_layout::WindowId;

use crate::hub::Address;
use crate::protocol::ToHub;

/// One connection from a window process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub u64);

/// Ties connections to windows. A connection belongs to a window once it
/// says hello.
#[derive(Debug, Clone, Default)]
pub struct Switchboard {
    windows: BTreeMap<ConnId, WindowId>,
}

impl Switchboard {
    /// The window a message comes from. A hello ties the connection to its
    /// window and drops an older connection of that window. A connection
    /// that has not said hello has no window: drop its messages.
    pub fn sender(&mut self, conn: ConnId, message: &ToHub) -> Option<WindowId> {
        if let ToHub::Hello { window } = message {
            self.windows.retain(|_, w| w != window);
            self.windows.insert(conn, *window);
        }
        self.windows.get(&conn).copied()
    }

    /// Forgets a connection that closed, and returns its window.
    pub fn gone(&mut self, conn: ConnId) -> Option<WindowId> {
        self.windows.remove(&conn)
    }

    /// True if a window has a connection.
    pub fn is_connected(&self, window: WindowId) -> bool {
        self.windows.values().any(|w| *w == window)
    }

    /// The connections a message to `to` goes to. The caller encodes the
    /// message once and writes the same line to each.
    pub fn recipients(&self, to: Address) -> Vec<ConnId> {
        self.windows
            .iter()
            .filter(|(_, window)| to.reaches(**window))
            .map(|(conn, _)| *conn)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(window: u32) -> ToHub {
        ToHub::Hello {
            window: WindowId(window),
        }
    }

    #[test]
    fn a_connection_has_a_window_after_hello() {
        let mut board = Switchboard::default();
        let focus = ToHub::Focused {
            panel: fusion_layout::PanelId(1),
        };
        assert_eq!(board.sender(ConnId(1), &focus), None);
        assert_eq!(board.sender(ConnId(1), &hello(2)), Some(WindowId(2)));
        assert_eq!(board.sender(ConnId(1), &focus), Some(WindowId(2)));
        assert!(board.is_connected(WindowId(2)));
        assert!(!board.is_connected(WindowId(3)));
    }

    #[test]
    fn a_new_connection_of_a_window_replaces_the_old_one() {
        let mut board = Switchboard::default();
        board.sender(ConnId(1), &hello(2));
        board.sender(ConnId(2), &hello(2));
        assert_eq!(board.recipients(Address::All), [ConnId(2)]);
        assert_eq!(board.gone(ConnId(1)), None);
        assert_eq!(board.gone(ConnId(2)), Some(WindowId(2)));
        assert!(!board.is_connected(WindowId(2)));
    }

    #[test]
    fn messages_reach_their_windows_only() {
        let mut board = Switchboard::default();
        board.sender(ConnId(7), &hello(1));
        board.sender(ConnId(8), &hello(2));
        assert_eq!(board.recipients(Address::All), [ConnId(7), ConnId(8)]);
        assert_eq!(board.recipients(Address::Window(WindowId(2))), [ConnId(8)]);
        assert!(board.recipients(Address::Window(WindowId(3))).is_empty());
    }
}
