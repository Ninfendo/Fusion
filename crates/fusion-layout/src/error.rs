//! Why a layout was refused.

use std::fmt;

use crate::{PanelId, WindowId};

/// Why a layout, or a change to it, was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    /// The file was written by a different format version.
    UnsupportedVersion {
        /// The version in the file.
        found: u32,
        /// The version this build reads.
        supported: u32,
    },
    /// The JSON could not be read or written.
    Json(String),
    /// A layout needs at least one window.
    NoWindows,
    /// Two windows have the same id.
    DuplicateWindow(WindowId),
    /// A window id that the layout does not have.
    UnknownWindow(WindowId),
    /// A panel id that the layout does not have.
    UnknownPanel(PanelId),
    /// A panel sits in more than one place.
    PanelPlacedTwice(PanelId),
    /// A panel is in the layout but in no window.
    PanelNotPlaced(PanelId),
    /// A tab group or a split has no children.
    EmptyContainer,
    /// A tab group's active tab is past its last tab.
    ActiveTabOutOfRange {
        /// The active index.
        active: usize,
        /// The number of tabs.
        tabs: usize,
    },
    /// A split share is zero, negative or not a number.
    BadShare(f32),
    /// A link has no target panels.
    LinkWithoutTargets(PanelId),
    /// A link points back at the panel it starts from.
    LinkToItself(PanelId),
    /// A link names the same target panel twice.
    LinkTargetTwice(PanelId),
    /// The next panel id is not above every panel id in use.
    NextIdTooLow {
        /// The next panel id in the layout.
        next: PanelId,
        /// The highest panel id in use.
        highest: PanelId,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "layout format {found} is not supported; this build reads format {supported}"
            ),
            Self::Json(message) => write!(f, "the layout file is not valid: {message}"),
            Self::NoWindows => f.write_str("a layout needs at least one window"),
            Self::DuplicateWindow(id) => write!(f, "window {} appears twice", id.0),
            Self::UnknownWindow(id) => write!(f, "there is no window {}", id.0),
            Self::UnknownPanel(id) => write!(f, "there is no panel {}", id.0),
            Self::PanelPlacedTwice(id) => write!(f, "panel {} is placed twice", id.0),
            Self::PanelNotPlaced(id) => write!(f, "panel {} is in no window", id.0),
            Self::EmptyContainer => f.write_str("a tab group or split has no children"),
            Self::ActiveTabOutOfRange { active, tabs } => {
                write!(f, "active tab {active} is out of range for {tabs} tabs")
            }
            Self::BadShare(share) => write!(f, "split share {share} must be above zero"),
            Self::LinkWithoutTargets(id) => write!(f, "a link from panel {} has no targets", id.0),
            Self::LinkToItself(id) => write!(f, "panel {} cannot link to itself", id.0),
            Self::LinkTargetTwice(id) => write!(f, "panel {} is a link target twice", id.0),
            Self::NextIdTooLow { next, highest } => write!(
                f,
                "the next panel id {} must be above the highest id in use, {}",
                next.0, highest.0
            ),
        }
    }
}

impl std::error::Error for LayoutError {}
