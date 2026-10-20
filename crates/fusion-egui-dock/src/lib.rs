//! Draws a Fusion dock tree with `egui_tiles`.
//!
//! The layout model ([`fusion_layout::Node`]) is what Fusion saves.
//! `egui_tiles` keeps its own tree while the user drags, splits and resizes.
//! [`to_tree`] and [`to_node`] convert between the two, so the saved format
//! never depends on `egui_tiles`.

use egui_tiles::{
    Behavior, Container, EditAction, LinearDir, SimplificationOptions, Tile, TileId, Tiles, Tree,
    UiResponse,
};
use fusion_layout::{Child, Direction, Node, PanelId};

/// The `egui_tiles` tree the dock draws. Its panes are panel ids.
pub type DockTree = Tree<PanelId>;

/// Builds the drawable tree for a window's layout.
pub fn to_tree(id: impl Into<egui::Id>, root: Option<&Node>) -> DockTree {
    match root {
        None => Tree::empty(id),
        Some(node) => {
            let mut tiles = Tiles::default();
            let root = insert(&mut tiles, node);
            Tree::new(id, root, tiles)
        }
    }
}

fn insert(tiles: &mut Tiles<PanelId>, node: &Node) -> TileId {
    match node {
        Node::Panel { id } => tiles.insert_pane(*id),
        Node::Tabs { active, children } => {
            let ids: Vec<TileId> = children.iter().map(|c| insert(tiles, c)).collect();
            let tabs = tiles.insert_tab_tile(ids.clone());
            if let (Some(Tile::Container(Container::Tabs(t))), Some(shown)) =
                (tiles.get_mut(tabs), ids.get(*active))
            {
                t.set_active(*shown);
            }
            tabs
        }
        Node::Split {
            direction,
            children,
        } => {
            let ids: Vec<TileId> = children.iter().map(|c| insert(tiles, &c.node)).collect();
            let split = match direction {
                Direction::Horizontal => tiles.insert_horizontal_tile(ids.clone()),
                Direction::Vertical => tiles.insert_vertical_tile(ids.clone()),
            };
            if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(split) {
                for (id, child) in ids.iter().zip(children) {
                    linear.shares.set_share(*id, child.share);
                }
            }
            split
        }
    }
}

/// Reads the layout model back from the drawable tree.
pub fn to_node(tree: &DockTree) -> Option<Node> {
    tree.root().and_then(|root| node_of(&tree.tiles, root))
}

fn node_of(tiles: &Tiles<PanelId>, id: TileId) -> Option<Node> {
    match tiles.get(id)? {
        Tile::Pane(panel) => Some(Node::panel(*panel)),
        Tile::Container(Container::Tabs(tabs)) => {
            let shown = tabs.active;
            let mut active = 0;
            let mut children = Vec::new();
            for child in &tabs.children {
                if let Some(node) = node_of(tiles, *child) {
                    if Some(*child) == shown {
                        active = children.len();
                    }
                    children.push(node);
                }
            }
            (!children.is_empty()).then_some(Node::Tabs { active, children })
        }
        Tile::Container(Container::Linear(linear)) => {
            let children: Vec<Child> = linear
                .children
                .iter()
                .filter_map(|child| {
                    node_of(tiles, *child).map(|node| Child {
                        share: linear.shares[*child],
                        node,
                    })
                })
                .collect();
            let direction = match linear.dir {
                LinearDir::Horizontal => Direction::Horizontal,
                LinearDir::Vertical => Direction::Vertical,
            };
            (!children.is_empty()).then_some(Node::Split {
                direction,
                children,
            })
        }
        // Fusion never creates grids; read one as tabs so nothing is lost.
        Tile::Container(Container::Grid(grid)) => {
            let children: Vec<Node> = grid
                .children()
                .filter_map(|child| node_of(tiles, *child))
                .collect();
            (!children.is_empty()).then_some(Node::tabs(children))
        }
    }
}

/// What the dock needs from the app to draw the panels.
pub trait PanelHost {
    /// The title on a panel's tab.
    fn title(&self, id: PanelId) -> String;
    /// Draws a panel's content.
    fn ui(&mut self, ui: &mut egui::Ui, id: PanelId);
}

/// What the user did to the dock in one frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockOutput {
    /// The tree changed: a drag, a resize, a tab switch or a close.
    pub edited: bool,
    /// Panels whose tab the user closed.
    pub closed: Vec<PanelId>,
}

/// Draws the dock and reports what the user changed.
pub fn show(tree: &mut DockTree, ui: &mut egui::Ui, host: &mut dyn PanelHost) -> DockOutput {
    let mut behavior = DockBehavior {
        host,
        output: DockOutput::default(),
    };
    tree.ui(&mut behavior, ui);
    behavior.output
}

struct DockBehavior<'a> {
    host: &'a mut dyn PanelHost,
    output: DockOutput,
}

impl Behavior<PanelId> for DockBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut PanelId) -> UiResponse {
        self.host.ui(ui, *pane);
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PanelId) -> egui::WidgetText {
        self.host.title(*pane).into()
    }

    fn is_tab_closable(&self, tiles: &Tiles<PanelId>, tile_id: TileId) -> bool {
        matches!(tiles.get(tile_id), Some(Tile::Pane(_)))
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<PanelId>, tile_id: TileId) -> bool {
        if let Some(Tile::Pane(panel)) = tiles.get(tile_id) {
            self.output.closed.push(*panel);
        }
        self.output.edited = true;
        true
    }

    fn on_edit(&mut self, _edit_action: EditAction) {
        self.output.edited = true;
    }

    fn simplification_options(&self) -> SimplificationOptions {
        // Every panel keeps a tab, so it can always be dragged by it.
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        }
    }
}

#[cfg(test)]
mod tests;
