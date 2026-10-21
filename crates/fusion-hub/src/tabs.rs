//! The hub across browser tabs.
//!
//! In the browser, the main tab runs the hub. The other tabs (popped-out
//! windows, or a second copy of the main tab) talk to it over a
//! BroadcastChannel, which hands every post to every other tab of the site.
//! Each post is one [`Envelope`] as JSON. Everything here is pure; the
//! browser calls live in the web app.

use fusion_layout::{Layout, PanelId, WindowId};
use serde::{Deserialize, Serialize};

use crate::hub::{Address, Hub};
use crate::protocol::{FromHub, ToHub};

/// One post on the channel between tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Envelope {
    /// From a tab to the hub.
    ToHub {
        /// The tab's window.
        from: WindowId,
        /// The message.
        message: ToHub,
    },
    /// From the hub to the tabs it names.
    FromHub {
        /// Who it is for.
        to: Address,
        /// The message.
        message: FromHub,
    },
    /// The hub tab started, for example after a reload: say hello again.
    HubStarted,
}

impl Envelope {
    /// The message in a post, if the post is for `window`.
    pub fn for_window(self, window: WindowId) -> Option<FromHub> {
        match self {
            Self::FromHub { to, message } if to.reaches(window) => Some(message),
            _ => None,
        }
    }
}

/// What the hub tab does after one message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabReply {
    /// Messages for the hub tab's own window.
    pub mine: Vec<FromHub>,
    /// Posts for the other tabs.
    pub post: Vec<Envelope>,
    /// The layout changed: save it.
    pub changed: bool,
}

/// The hub, running in the main tab.
#[derive(Debug, Clone)]
pub struct TabHub {
    hub: Hub,
}

impl TabHub {
    /// A hub for `layout`. A browser cannot open windows without a click,
    /// so windows left over from last time come back into the main window.
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
        Self {
            hub: Hub::new(layout),
        }
    }

    /// The layout.
    pub fn layout(&self) -> &Layout {
        self.hub.layout()
    }

    /// Handles a message from the tab of `sender`, this tab or another.
    /// Closing the main window ends every tab.
    pub fn handle(&mut self, sender: WindowId, message: ToHub) -> TabReply {
        let reply = self.hub.handle(sender, message);
        let mut out = TabReply::default();
        for output in reply.messages {
            if output.to == Address::All && matches!(output.message, FromHub::Layout { .. }) {
                out.changed = true;
            }
            if output.to.reaches(WindowId::MAIN) {
                out.mine.push(output.message.clone());
            }
            // Posted even when only for the main window: a second copy of
            // the main tab shows the same window.
            out.post.push(Envelope::FromHub {
                to: output.to,
                message: output.message,
            });
        }
        if reply.quit {
            out.post.push(Envelope::FromHub {
                to: Address::All,
                message: FromHub::Quit,
            });
        }
        out
    }
}

/// How long a browser window opened for a popping-out panel waits for the
/// hub before it closes again.
pub const POPUP_WAIT_SECONDS: f64 = 5.0;

/// What to do with a browser window opened for a panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup<H> {
    /// Load the window's page into it.
    Show(H, WindowId),
    /// The hub did not move the panel: close it.
    Close(H),
}

/// Browser windows opened for panels that are popping out, until the hub
/// says where each panel went.
///
/// A browser opens a window only right after a click, so the tab opens an
/// empty one at once and fills it when the hub answers.
#[derive(Debug, Clone)]
pub struct PendingPopups<H> {
    waiting: Vec<(PanelId, H, f64)>,
}

impl<H> Default for PendingPopups<H> {
    fn default() -> Self {
        Self {
            waiting: Vec::new(),
        }
    }
}

impl<H> PendingPopups<H> {
    /// Remembers the window opened at `now` (in seconds) for `panel`.
    pub fn add(&mut self, panel: PanelId, handle: H, now: f64) {
        self.waiting.push((panel, handle, now));
    }

    /// True if no window waits.
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Settles the waiting windows against the latest `layout`. A panel that
    /// moved out of `mine` shows in its new window; a window that waited
    /// longer than [`POPUP_WAIT_SECONDS`] closes. The rest keep waiting:
    /// the layout may be from before the hub saw the pop out.
    pub fn settle(&mut self, layout: Option<&Layout>, mine: WindowId, now: f64) -> Vec<Popup<H>> {
        let mut done = Vec::new();
        let mut keep = Vec::new();
        for (panel, handle, opened) in self.waiting.drain(..) {
            let moved_to = layout
                .and_then(|l| l.window_of(panel))
                .filter(|window| *window != mine);
            match moved_to {
                Some(window) => done.push(Popup::Show(handle, window)),
                None if now - opened > POPUP_WAIT_SECONDS => done.push(Popup::Close(handle)),
                None => keep.push((panel, handle, opened)),
            }
        }
        self.waiting = keep;
        done
    }
}

/// The window a tab shows, from its page address: `?window=N`, or the main
/// window.
pub fn window_from_query(query: &str) -> WindowId {
    query
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("window="))
        .and_then(|n| n.parse().ok())
        .map_or(WindowId::MAIN, WindowId)
}

/// The page address of another window of the same page: the same query,
/// with `window` set. `?server=x` and window 2 give `?server=x&window=2`,
/// so a popped-out tab keeps what the page was started with.
pub fn query_for_window(query: &str, window: WindowId) -> String {
    let window = format!("window={window}");
    let pairs: Vec<&str> = query
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty() && !pair.starts_with("window="))
        .chain([window.as_str()])
        .collect();
    format!("?{}", pairs.join("&"))
}

#[cfg(test)]
mod tests;
