//! The panel kinds, and the layout a first start shows.
//!
//! The launcher needs the default layout without drawing anything, so it
//! lives here and not in the UI crate.

use fusion_layout::{Child, ContextType, Direction, Layout, Link, LinkMode, Node, Panel, PanelId};

/// The kind names, as the layout stores them.
pub mod kind {
    /// The watchlist.
    pub const WATCHLIST: &str = "watchlist";
    /// The L2 ladder.
    pub const L2: &str = "l2";
    /// The event log.
    pub const LOG: &str = "log";
}

/// A panel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelKind {
    /// The kind as the layout stores it.
    pub kind: &'static str,
    /// The title a new panel of this kind gets.
    pub title: &'static str,
}

/// Every panel type, in menu order.
pub const KINDS: [PanelKind; 3] = [
    PanelKind {
        kind: kind::WATCHLIST,
        title: "Watchlist",
    },
    PanelKind {
        kind: kind::L2,
        title: "L2",
    },
    PanelKind {
        kind: kind::LOG,
        title: "Log",
    },
];

/// The title of a kind; a kind this build does not know is its own title.
pub fn title_of(kind: &str) -> &str {
    KINDS
        .iter()
        .find(|k| k.kind == kind)
        .map_or(kind, |k| k.title)
}

/// The layout a first start shows: a watchlist on the left, an L2 ladder
/// and the log on the right, and the watchlist linked to the ladder.
pub fn default_layout() -> Layout {
    let mut layout = Layout::new("Fusion");
    for (id, kind) in (1..).zip(KINDS) {
        layout
            .panels
            .insert(PanelId(id), Panel::new(kind.kind, kind.title));
    }
    layout.next_panel_id = PanelId(KINDS.len() as u64 + 1);
    let tabs = |id| Node::tabs(vec![Node::panel(PanelId(id))]);
    layout.windows[0].root = Some(Node::Split {
        direction: Direction::Horizontal,
        children: vec![
            Child {
                share: 3.0,
                node: tabs(1),
            },
            Child {
                share: 2.0,
                node: Node::Split {
                    direction: Direction::Vertical,
                    children: vec![
                        Child {
                            share: 3.0,
                            node: tabs(2),
                        },
                        Child {
                            share: 1.0,
                            node: tabs(3),
                        },
                    ],
                },
            },
        ],
    });
    layout.links.push(Link {
        from: PanelId(1),
        context: ContextType::instrument(),
        to: vec![PanelId(2)],
        mode: LinkMode::All,
        gesture: None,
    });
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_unique_name() {
        for (i, a) in KINDS.iter().enumerate() {
            for b in &KINDS[i + 1..] {
                assert_ne!(a.kind, b.kind);
            }
        }
    }

    #[test]
    fn kinds_have_titles() {
        assert_eq!(title_of(kind::L2), "L2");
        assert_eq!(title_of("heatmap"), "heatmap");
    }

    #[test]
    fn the_default_layout_is_valid_and_has_every_kind() {
        let layout = default_layout();
        assert_eq!(layout.validate(), Ok(()));
        let kinds: Vec<&str> = layout.panels.values().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, [kind::WATCHLIST, kind::L2, kind::LOG]);
        assert_eq!(layout.links.len(), 1);
    }
}
