//! The bus a window talks to the hub through, and the in-process version.

use fusion_layout::{Layout, WindowId};

use crate::hub::Hub;
use crate::protocol::{FromHub, ToHub};

/// How a window talks to the hub.
pub trait Bus {
    /// Sends a message to the hub.
    fn send(&mut self, message: ToHub);
    /// Takes the messages that arrived for this window.
    fn poll(&mut self) -> Vec<FromHub>;
    /// True if the hub can open windows of its own, as the launcher can.
    /// False when the hub runs inside a single window.
    fn can_open_windows(&self) -> bool;
}

/// The hub running inside the window: one window, no other processes.
#[derive(Debug, Clone)]
pub struct LocalBus {
    hub: Hub,
    inbox: Vec<FromHub>,
}

impl LocalBus {
    /// A bus around a layout. Panels in other windows come back into the
    /// main window, because a single window cannot show other windows.
    pub fn new(mut layout: Layout) -> Self {
        let others: Vec<WindowId> = layout
            .windows
            .iter()
            .map(|w| w.id)
            .filter(|id| *id != WindowId::MAIN)
            .collect();
        for window in others {
            // A window that cannot close is already gone; nothing is lost.
            let _ignored = layout.close_window(window);
        }
        let mut bus = Self {
            hub: Hub::new(layout),
            inbox: Vec::new(),
        };
        bus.send(ToHub::Hello {
            window: WindowId::MAIN,
        });
        bus
    }

    /// The hub.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }
}

impl Bus for LocalBus {
    fn send(&mut self, message: ToHub) {
        let reply = self.hub.handle(WindowId::MAIN, message);
        self.inbox.extend(
            reply
                .messages
                .into_iter()
                .filter(|out| out.to.reaches(WindowId::MAIN))
                .map(|out| out.message),
        );
    }

    fn poll(&mut self) -> Vec<FromHub> {
        std::mem::take(&mut self.inbox)
    }

    fn can_open_windows(&self) -> bool {
        false
    }
}
