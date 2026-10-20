//! Moving panels between windows: pop out, pop in, close a window.

use serde::{Deserialize, Serialize};

use crate::{Direction, Layout, LayoutError, Node, PanelId, Window, WindowId};

/// Where a popped-out panel came from, so pop in can put it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// The window it was in.
    pub window: WindowId,
    /// A neighbour tab in its old tab group, used to find that group again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beside: Option<PanelId>,
    /// Its position in that tab group.
    pub index: usize,
}

impl Layout {
    /// The window a panel is in.
    pub fn window_of(&self, panel: PanelId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| {
                w.root
                    .as_ref()
                    .is_some_and(|r| r.panel_ids().contains(&panel))
            })
            .map(|w| w.id)
    }

    /// Moves a panel into a new window of its own and returns that window's
    /// id. The panel remembers where it came from.
    pub fn pop_out(&mut self, panel: PanelId) -> Result<WindowId, LayoutError> {
        let from = self
            .window_of(panel)
            .ok_or(LayoutError::UnknownPanel(panel))?;
        if from != WindowId::MAIN && self.panels_in(from).len() == 1 {
            return Err(LayoutError::AlreadyAlone(panel));
        }
        let (beside, index) = self
            .window(from)
            .and_then(|w| w.root.as_ref())
            .and_then(|root| root.tab_position(panel))
            .unwrap_or((None, 0));
        self.take_panel(from, panel);
        let id = self.next_window_id();
        let title = self
            .panels
            .get(&panel)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        self.windows.push(Window {
            id,
            title,
            bounds: None,
            root: Some(Node::tabs(vec![Node::panel(panel)])),
        });
        if let Some(p) = self.panels.get_mut(&panel) {
            p.origin = Some(Origin {
                window: from,
                beside,
                index,
            });
        }
        Ok(id)
    }

    /// Moves a popped-out panel back where it came from: next to its old
    /// neighbour, at its old position. If the neighbour is gone it joins the
    /// first tab group of its old window, or of the main window. Returns the
    /// window it went to.
    pub fn pop_in(&mut self, panel: PanelId) -> Result<WindowId, LayoutError> {
        let current = self
            .window_of(panel)
            .ok_or(LayoutError::UnknownPanel(panel))?;
        let origin = self.panels.get_mut(&panel).and_then(|p| p.origin.take());
        if origin.is_none() && current == WindowId::MAIN {
            return Err(LayoutError::NotPoppedOut(panel));
        }
        self.take_panel(current, panel);
        let target = origin
            .map(|o| o.window)
            .filter(|w| self.window(*w).is_some())
            .unwrap_or(WindowId::MAIN);
        self.place(target, panel, origin);
        Ok(target)
    }

    /// Closes a window. Its panels pop back in; nothing is lost. The main
    /// window cannot be closed this way.
    pub fn close_window(&mut self, window: WindowId) -> Result<(), LayoutError> {
        if window == WindowId::MAIN {
            return Err(LayoutError::MainWindowStays);
        }
        if self.window(window).is_none() {
            return Err(LayoutError::UnknownWindow(window));
        }
        for panel in self.panels_in(window) {
            self.pop_in(panel)?;
        }
        self.windows.retain(|w| w.id != window);
        Ok(())
    }

    /// One above the highest window id.
    fn next_window_id(&self) -> WindowId {
        WindowId(self.windows.iter().map(|w| w.id.0).max().unwrap_or(0) + 1)
    }

    /// Removes a panel from a window's tree. A window other than the main
    /// one that is left empty goes away.
    fn take_panel(&mut self, window: WindowId, panel: PanelId) {
        if let Some(w) = self.window_mut(window) {
            w.root = w
                .root
                .take()
                .and_then(|root| root.without_panel(panel).simplified());
        }
        self.windows
            .retain(|w| w.id == WindowId::MAIN || w.root.is_some());
    }

    /// Puts a panel into a window: next to its old neighbour if that is in
    /// this window, else into the first tab group, else beside the tree.
    fn place(&mut self, window: WindowId, panel: PanelId, origin: Option<Origin>) {
        let Some(w) = self.window_mut(window) else {
            return;
        };
        let beside = origin.and_then(|o| o.beside.map(|b| (b, o.index)));
        w.root = Some(match w.root.take() {
            None => Node::tabs(vec![Node::panel(panel)]),
            Some(mut root) => {
                let placed = beside.is_some_and(|(b, i)| root.insert_beside(b, panel, i))
                    || root.add_to_first_tabs(panel);
                if placed {
                    root
                } else {
                    Node::split(
                        Direction::Horizontal,
                        vec![root, Node::tabs(vec![Node::panel(panel)])],
                    )
                }
            }
        });
    }
}
