//! The dock tree inside one window.

use serde::{Deserialize, Serialize};

use crate::{LayoutError, PanelId};

/// Which way a split lays out its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Side by side, left to right.
    Horizontal,
    /// Stacked, top to bottom.
    Vertical,
}

/// One child of a split, with its share of the space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Child {
    /// The share of the split's space. Shares are relative: 1 and 3 give a
    /// quarter and three quarters.
    pub share: f32,
    /// The child.
    pub node: Node,
}

/// A node of the dock tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    /// A panel.
    Panel {
        /// The panel's id.
        id: PanelId,
    },
    /// A tab group: one child shows at a time.
    Tabs {
        /// The index of the child that shows.
        active: usize,
        /// The tabs, in order.
        children: Vec<Node>,
    },
    /// A split: all children show, side by side or stacked.
    Split {
        /// Side by side or stacked.
        direction: Direction,
        /// The children and their shares.
        children: Vec<Child>,
    },
}

impl Node {
    /// A panel leaf.
    pub fn panel(id: PanelId) -> Self {
        Self::Panel { id }
    }

    /// A tab group showing its first child.
    pub fn tabs(children: Vec<Node>) -> Self {
        Self::Tabs {
            active: 0,
            children,
        }
    }

    /// A split with equal shares.
    pub fn split(direction: Direction, children: Vec<Node>) -> Self {
        Self::Split {
            direction,
            children: children
                .into_iter()
                .map(|node| Child { share: 1.0, node })
                .collect(),
        }
    }

    /// Every panel id under this node, depth first.
    pub fn panel_ids(&self) -> Vec<PanelId> {
        let mut ids = Vec::new();
        self.collect_panel_ids(&mut ids);
        ids
    }

    fn collect_panel_ids(&self, ids: &mut Vec<PanelId>) {
        match self {
            Self::Panel { id } => ids.push(*id),
            Self::Tabs { children, .. } => {
                children.iter().for_each(|c| c.collect_panel_ids(ids));
            }
            Self::Split { children, .. } => {
                children.iter().for_each(|c| c.node.collect_panel_ids(ids));
            }
        }
    }

    /// Checks this subtree: no empty groups, an active tab in range, and
    /// every share above zero.
    pub fn validate(&self) -> Result<(), LayoutError> {
        match self {
            Self::Panel { .. } => Ok(()),
            Self::Tabs { active, children } => {
                if children.is_empty() {
                    return Err(LayoutError::EmptyContainer);
                }
                if *active >= children.len() {
                    return Err(LayoutError::ActiveTabOutOfRange {
                        active: *active,
                        tabs: children.len(),
                    });
                }
                children.iter().try_for_each(Self::validate)
            }
            Self::Split { children, .. } => {
                if children.is_empty() {
                    return Err(LayoutError::EmptyContainer);
                }
                for child in children {
                    if !(child.share.is_finite() && child.share > 0.0) {
                        return Err(LayoutError::BadShare(child.share));
                    }
                    child.node.validate()?;
                }
                Ok(())
            }
        }
    }

    /// Adds a panel as the last tab of the first tab group, depth first.
    /// Returns false if the tree has no tab group.
    pub(crate) fn add_to_first_tabs(&mut self, id: PanelId) -> bool {
        match self {
            Self::Panel { .. } => false,
            Self::Tabs { children, .. } => {
                children.push(Self::panel(id));
                true
            }
            Self::Split { children, .. } => children
                .iter_mut()
                .any(|child| child.node.add_to_first_tabs(id)),
        }
    }

    /// This subtree without the panel. Containers may be left empty;
    /// [`Node::simplified`] removes them.
    pub(crate) fn without_panel(self, id: PanelId) -> Self {
        let is_it = |node: &Node| matches!(node, Self::Panel { id: p } if *p == id);
        match self {
            Self::Tabs { active, children } => Self::Tabs {
                // Closing a tab left of the active one keeps the same tab showing.
                active: active - children.iter().take(active).filter(|c| is_it(c)).count(),
                children: children
                    .into_iter()
                    .filter(|c| !matches!(c, Self::Panel { id: p } if *p == id))
                    .map(|c| c.without_panel(id))
                    .collect(),
            },
            Self::Split {
                direction,
                children,
            } => Self::Split {
                direction,
                children: children
                    .into_iter()
                    .filter(|c| !matches!(c.node, Self::Panel { id: p } if p == id))
                    .map(|c| Child {
                        share: c.share,
                        node: c.node.without_panel(id),
                    })
                    .collect(),
            },
            panel @ Self::Panel { .. } => panel,
        }
    }

    /// Tidies the tree: empty groups and splits go, a split with one child
    /// becomes that child, and an active tab index past the end moves to the
    /// last tab. A tab group with one tab stays, so its tab can still be
    /// dragged. Returns `None` if nothing is left.
    pub fn simplified(self) -> Option<Self> {
        match self {
            panel @ Self::Panel { .. } => Some(panel),
            Self::Tabs { active, children } => {
                let children: Vec<Node> =
                    children.into_iter().filter_map(Self::simplified).collect();
                (!children.is_empty()).then(|| Self::Tabs {
                    active: active.min(children.len() - 1),
                    children,
                })
            }
            Self::Split {
                direction,
                children,
            } => {
                let mut children: Vec<Child> = children
                    .into_iter()
                    .filter_map(|c| {
                        c.node.simplified().map(|node| Child {
                            share: c.share,
                            node,
                        })
                    })
                    .collect();
                match children.len() {
                    0 => None,
                    1 => children.pop().map(|c| c.node),
                    _ => Some(Self::Split {
                        direction,
                        children,
                    }),
                }
            }
        }
    }
}
