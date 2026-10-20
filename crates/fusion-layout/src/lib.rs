//! The layout of a Fusion workspace: the windows, the dock tree in each
//! window, and the panels the trees point to.
//!
//! This crate has no UI library in it. `fusion-egui-dock` draws a layout with
//! egui; another UI library could draw the same layout. A layout is saved as
//! versioned JSON. Loading checks the version first and the structure next, so
//! a broken file is refused instead of drawing a broken screen.

mod error;
mod links;
mod node;
mod windows;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use error::LayoutError;
pub use links::{ContextType, Gesture, Link, LinkColour, LinkMode};
pub use node::{Child, Direction, Node};
pub use windows::Origin;

/// The id of a panel. Unique within a layout, and never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PanelId(pub u64);

/// The id of a window. The main window is [`WindowId::MAIN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowId(pub u32);

impl WindowId {
    /// The main window, which always exists.
    pub const MAIN: Self = Self(1);
}

/// One panel: what it shows and its own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// The panel type, as the panel registry knows it: for example
    /// `"watchlist"`. A kind this build does not know is kept, so a newer
    /// layout survives a round trip through an older build.
    pub kind: String,
    /// The title on the panel's tab.
    pub title: String,
    /// The panel's own settings. Each panel kind reads and checks its own.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub config: serde_json::Value,
    /// Where the panel was before it was popped out into its own window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Panel {
    /// A panel with no settings.
    pub fn new(kind: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            config: serde_json::Value::Null,
            origin: None,
        }
    }
}

/// Where a window sits on screen, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// One window and its dock tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// The window's id.
    pub id: WindowId,
    /// The window title.
    pub title: String,
    /// Where the window was last. `None` lets the system choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    /// The dock tree. `None` for an empty window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<Node>,
}

/// A whole workspace layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The format version. See [`Layout::SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The windows. The first is the main window.
    pub windows: Vec<Window>,
    /// Every panel, by id. Each one sits in exactly one window's tree.
    pub panels: BTreeMap<PanelId, Panel>,
    /// The id the next new panel gets. It only goes up, so an id is never
    /// reused, even after its panel is closed.
    pub next_panel_id: PanelId,
    /// Links between panels, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Colour groups: panels in the same group follow each other.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<PanelId, LinkColour>,
}

/// Just the version, read before anything else.
#[derive(Deserialize)]
struct VersionOnly {
    schema_version: u32,
}

impl Layout {
    /// The format version this build writes and reads.
    pub const SCHEMA_VERSION: u32 = 2;

    /// The oldest format this build still reads. Format 1 had no links and
    /// no colour groups; they load empty.
    pub const OLDEST_READABLE: u32 = 1;

    /// A layout with one empty main window.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            windows: vec![Window {
                id: WindowId::MAIN,
                title: title.into(),
                bounds: None,
                root: None,
            }],
            panels: BTreeMap::new(),
            next_panel_id: PanelId(1),
            links: Vec::new(),
            groups: BTreeMap::new(),
        }
    }

    /// A window by id.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// A window by id, to change it.
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// A panel by id.
    pub fn panel(&self, id: PanelId) -> Option<&Panel> {
        self.panels.get(&id)
    }

    /// The panels in one window, in tree order.
    pub fn panels_in(&self, window: WindowId) -> Vec<PanelId> {
        self.window(window)
            .and_then(|w| w.root.as_ref())
            .map(Node::panel_ids)
            .unwrap_or_default()
    }

    /// Adds a panel to a window, as a new tab in the window's first tab
    /// group. An empty window gets a tab group with just this panel.
    pub fn add_panel(&mut self, window: WindowId, panel: Panel) -> Result<PanelId, LayoutError> {
        let id = self.next_panel_id;
        let target = self
            .window_mut(window)
            .ok_or(LayoutError::UnknownWindow(window))?;
        target.root = Some(match target.root.take() {
            None => Node::tabs(vec![Node::panel(id)]),
            Some(mut root) => {
                if root.add_to_first_tabs(id) {
                    root
                } else {
                    // No tab group yet: put the old tree and the new panel side by side.
                    Node::split(
                        Direction::Horizontal,
                        vec![root, Node::tabs(vec![Node::panel(id)])],
                    )
                }
            }
        });
        self.panels.insert(id, panel);
        self.next_panel_id = PanelId(id.0 + 1);
        Ok(id)
    }

    /// Removes a panel from its window and from the layout, then tidies the
    /// tree: empty groups go, and a split with one child becomes that child.
    pub fn close_panel(&mut self, id: PanelId) -> Result<Panel, LayoutError> {
        let panel = self
            .panels
            .remove(&id)
            .ok_or(LayoutError::UnknownPanel(id))?;
        for window in &mut self.windows {
            window.root = window
                .root
                .take()
                .and_then(|root| root.without_panel(id).simplified());
        }
        self.links.retain_mut(|link| {
            link.to.retain(|target| *target != id);
            link.from != id && !link.to.is_empty()
        });
        self.groups.remove(&id);
        Ok(panel)
    }

    /// Puts a panel in a colour group, or takes it out with `None`.
    pub fn set_group(
        &mut self,
        panel: PanelId,
        colour: Option<LinkColour>,
    ) -> Result<(), LayoutError> {
        if !self.panels.contains_key(&panel) {
            return Err(LayoutError::UnknownPanel(panel));
        }
        match colour {
            Some(colour) => self.groups.insert(panel, colour),
            None => self.groups.remove(&panel),
        };
        Ok(())
    }

    /// Adds a link, after checking it.
    pub fn add_link(&mut self, link: Link) -> Result<(), LayoutError> {
        self.links.push(link);
        if let Err(err) = self.validate_links() {
            self.links.pop();
            return Err(err);
        }
        Ok(())
    }

    /// Removes the link at `index`, if there is one.
    pub fn remove_link(&mut self, index: usize) -> Option<Link> {
        (index < self.links.len()).then(|| self.links.remove(index))
    }

    /// Checks the whole layout: the version, unique window ids, every panel
    /// placed exactly once, and a sound tree in every window.
    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(LayoutError::UnsupportedVersion {
                found: self.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if self.windows.is_empty() {
            return Err(LayoutError::NoWindows);
        }
        if let Some(highest) = self.panels.keys().next_back()
            && *highest >= self.next_panel_id
        {
            return Err(LayoutError::NextIdTooLow {
                next: self.next_panel_id,
                highest: *highest,
            });
        }
        let mut window_ids = BTreeSet::new();
        let mut placed = BTreeSet::new();
        for window in &self.windows {
            if !window_ids.insert(window.id) {
                return Err(LayoutError::DuplicateWindow(window.id));
            }
            if let Some(root) = &window.root {
                root.validate()?;
                self.place_panels(root, &mut placed)?;
            }
        }
        if let Some(id) = self.panels.keys().find(|id| !placed.contains(id)) {
            return Err(LayoutError::PanelNotPlaced(*id));
        }
        self.validate_links()
    }

    /// Checks links and groups: known panels, at least one target, no link
    /// to its own panel, no target twice.
    fn validate_links(&self) -> Result<(), LayoutError> {
        let known = |id: &PanelId| {
            if self.panels.contains_key(id) {
                Ok(())
            } else {
                Err(LayoutError::UnknownPanel(*id))
            }
        };
        for link in &self.links {
            known(&link.from)?;
            if link.to.is_empty() {
                return Err(LayoutError::LinkWithoutTargets(link.from));
            }
            let mut seen = BTreeSet::new();
            for target in &link.to {
                known(target)?;
                if *target == link.from {
                    return Err(LayoutError::LinkToItself(link.from));
                }
                if !seen.insert(*target) {
                    return Err(LayoutError::LinkTargetTwice(*target));
                }
            }
        }
        self.groups.keys().try_for_each(known)
    }

    /// Records the panels of one tree, refusing unknown and doubled ones.
    fn place_panels(&self, root: &Node, placed: &mut BTreeSet<PanelId>) -> Result<(), LayoutError> {
        for id in root.panel_ids() {
            if !self.panels.contains_key(&id) {
                return Err(LayoutError::UnknownPanel(id));
            }
            if !placed.insert(id) {
                return Err(LayoutError::PanelPlacedTwice(id));
            }
        }
        Ok(())
    }

    /// The layout as pretty JSON.
    pub fn to_json(&self) -> Result<String, LayoutError> {
        serde_json::to_string_pretty(self).map_err(|e| LayoutError::Json(e.to_string()))
    }

    /// Reads a layout from JSON and checks it.
    pub fn from_json(text: &str) -> Result<Self, LayoutError> {
        let version: VersionOnly =
            serde_json::from_str(text).map_err(|e| LayoutError::Json(e.to_string()))?;
        if !(Self::OLDEST_READABLE..=Self::SCHEMA_VERSION).contains(&version.schema_version) {
            return Err(LayoutError::UnsupportedVersion {
                found: version.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        let mut layout: Self =
            serde_json::from_str(text).map_err(|e| LayoutError::Json(e.to_string()))?;
        // Older formats only lack fields that load as empty: upgrade in place.
        layout.schema_version = Self::SCHEMA_VERSION;
        layout.validate()?;
        Ok(layout)
    }
}

#[cfg(test)]
mod tests;
