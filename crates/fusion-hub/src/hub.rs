//! The hub state machine: it owns the layout and the link router, and
//! answers each message with what to send where.

use fusion_client_core::links::{Context, LinkRouter};
use fusion_layout::{Gesture, Layout, LayoutError, Node, Panel, PanelId, WindowId};
use serde::{Deserialize, Serialize};

use crate::protocol::{Fdc3Context, FromHub, ToHub};

/// Who a hub message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Address {
    /// Every window.
    All,
    /// One window.
    Window(WindowId),
}

impl Address {
    /// True if a message to this address reaches `window`.
    pub fn reaches(self, window: WindowId) -> bool {
        match self {
            Self::All => true,
            Self::Window(w) => w == window,
        }
    }
}

/// One message to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Who it is for.
    pub to: Address,
    /// The message.
    pub message: FromHub,
}

/// What the hub wants done after one message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    /// Messages to send.
    pub messages: Vec<Output>,
    /// Windows that now exist and did not before: start their processes.
    pub opened: Vec<WindowId>,
    /// Windows that existed and no longer do: stop their processes.
    pub closed: Vec<WindowId>,
    /// The main window was closed: end the app.
    pub quit: bool,
}

impl Reply {
    fn send(&mut self, to: Address, message: FromHub) {
        self.messages.push(Output { to, message });
    }
}

/// The hub.
#[derive(Debug, Clone)]
pub struct Hub {
    layout: Layout,
    router: LinkRouter,
}

impl Hub {
    /// A hub for a layout.
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            router: LinkRouter::default(),
        }
    }

    /// The layout as the hub holds it.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The window ids, in layout order.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.layout.windows.iter().map(|w| w.id).collect()
    }

    /// Handles one message from `sender`.
    pub fn handle(&mut self, sender: WindowId, message: ToHub) -> Reply {
        let before = self.window_ids();
        let mut reply = Reply::default();
        let changed = self.dispatch(sender, message, &mut reply);
        if changed {
            reply.send(
                Address::All,
                FromHub::Layout {
                    layout: self.layout.clone(),
                },
            );
        }
        let after = self.window_ids();
        reply.opened = after
            .iter()
            .filter(|w| !before.contains(w))
            .copied()
            .collect();
        reply.closed = before
            .iter()
            .filter(|w| !after.contains(w))
            .copied()
            .collect();
        reply
    }

    /// Applies one message. Returns true if the layout changed.
    fn dispatch(&mut self, sender: WindowId, message: ToHub, reply: &mut Reply) -> bool {
        match message {
            ToHub::Hello { window } => self.hello(window, reply),
            ToHub::WindowEdited {
                window,
                root,
                closed,
            } => self.window_edited(window, root, &closed, reply),
            ToHub::AddPanel {
                window,
                kind,
                title,
            } => self.apply(sender, reply, |l| {
                l.add_panel(window, Panel::new(kind, title)).map(drop)
            }),
            ToHub::SetGroup { panel, colour } => {
                self.apply(sender, reply, |l| l.set_group(panel, colour))
            }
            ToHub::AddLink { link } => self.links_change(sender, reply, |l| l.add_link(link)),
            ToHub::RemoveLink { index } => self.links_change(sender, reply, |l| {
                l.remove_link(index)
                    .map(drop)
                    .ok_or(LayoutError::UnknownLink(index))
            }),
            ToHub::ReplaceLayout { layout } => self.replace(sender, layout, reply),
            ToHub::PopOut { panel } => self.apply(sender, reply, |l| l.pop_out(panel).map(drop)),
            ToHub::PopIn { panel } => self.apply(sender, reply, |l| l.pop_in(panel).map(drop)),
            ToHub::CloseWindow { window } => self.close_window(sender, window, reply),
            ToHub::Select {
                from,
                context,
                gesture,
            } => self.select(from, context, gesture, reply),
            ToHub::Focused { panel } => {
                self.router.focused(panel);
                false
            }
        }
    }

    /// Runs a layout change; on error the sender hears why.
    fn apply(
        &mut self,
        sender: WindowId,
        reply: &mut Reply,
        change: impl FnOnce(&mut Layout) -> Result<(), LayoutError>,
    ) -> bool {
        match change(&mut self.layout) {
            Ok(()) => true,
            Err(err) => {
                reply.send(
                    Address::Window(sender),
                    FromHub::Notice {
                        text: format!("Refused: {err}"),
                    },
                );
                false
            }
        }
    }

    /// A change to the links: round-robin turns start over.
    fn links_change(
        &mut self,
        sender: WindowId,
        reply: &mut Reply,
        change: impl FnOnce(&mut Layout) -> Result<(), LayoutError>,
    ) -> bool {
        let changed = self.apply(sender, reply, change);
        if changed {
            self.router.links_changed();
        }
        changed
    }

    /// A window started: send it the layout and its panels' last contexts.
    /// A window that is not in the layout is told to quit.
    fn hello(&mut self, window: WindowId, reply: &mut Reply) -> bool {
        if self.layout.window(window).is_none() {
            reply.send(Address::Window(window), FromHub::Quit);
            return false;
        }
        reply.send(
            Address::Window(window),
            FromHub::Layout {
                layout: self.layout.clone(),
            },
        );
        for panel in self.layout.panels_in(window) {
            if let Some(context) = self.router.last_context(panel) {
                reply.send(
                    Address::Window(window),
                    FromHub::Deliver {
                        panel,
                        context: Fdc3Context::from(context),
                    },
                );
            }
        }
        false
    }

    /// The user edited a window's dock. Closed panels leave the layout. The
    /// new tree must hold exactly the window's panels; if not, the window
    /// gets the layout back to resynchronise.
    fn window_edited(
        &mut self,
        window: WindowId,
        root: Option<Node>,
        closed: &[PanelId],
        reply: &mut Reply,
    ) -> bool {
        let mut changed = false;
        for panel in closed {
            if self.layout.close_panel(*panel).is_ok() {
                self.router.forget(*panel);
                changed = true;
            }
        }
        let mut now = self.layout.panels_in(window);
        let mut new = root.as_ref().map(Node::panel_ids).unwrap_or_default();
        now.sort();
        new.sort();
        let sound = root.as_ref().is_none_or(|r| r.validate().is_ok());
        match self.layout.window_mut(window) {
            Some(w) if now == new && sound => {
                w.root = root;
                true
            }
            _ => {
                reply.send(
                    Address::Window(window),
                    FromHub::Layout {
                        layout: self.layout.clone(),
                    },
                );
                changed
            }
        }
    }

    /// Replaces the layout; the router starts over.
    fn replace(&mut self, sender: WindowId, layout: Layout, reply: &mut Reply) -> bool {
        let changed = self.apply(sender, reply, |current| {
            layout.validate()?;
            *current = layout;
            Ok(())
        });
        if changed {
            self.router = LinkRouter::default();
        }
        changed
    }

    fn close_window(&mut self, sender: WindowId, window: WindowId, reply: &mut Reply) -> bool {
        if window == WindowId::MAIN {
            reply.quit = true;
            return false;
        }
        self.apply(sender, reply, |l| l.close_window(window))
    }

    /// Sends a selection to each linked panel, in that panel's window.
    fn select(
        &mut self,
        from: PanelId,
        context: Fdc3Context,
        gesture: Option<Gesture>,
        reply: &mut Reply,
    ) -> bool {
        let context = Context::from(context);
        for target in self.router.route(&self.layout, from, &context, gesture) {
            if let Some(window) = self.layout.window_of(target) {
                reply.send(
                    Address::Window(window),
                    FromHub::Deliver {
                        panel: target,
                        context: Fdc3Context::from(&context),
                    },
                );
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
