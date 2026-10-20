//! Why a layout was refused.

use crate::{PanelId, WindowId};

/// Why a layout, or a change to it, was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LayoutError {
    /// The file was written by a different format version.
    #[error("layout format {found} is not supported; this build reads format {supported}")]
    UnsupportedVersion {
        /// The version in the file.
        found: u32,
        /// The version this build reads.
        supported: u32,
    },
    /// The JSON could not be read or written.
    #[error("the layout file is not valid: {0}")]
    Json(String),
    /// A layout needs at least one window.
    #[error("a layout needs at least one window")]
    NoWindows,
    /// Two windows have the same id.
    #[error("window {0} appears twice")]
    DuplicateWindow(WindowId),
    /// A window id that the layout does not have.
    #[error("there is no window {0}")]
    UnknownWindow(WindowId),
    /// A panel id that the layout does not have.
    #[error("there is no panel {0}")]
    UnknownPanel(PanelId),
    /// A panel sits in more than one place.
    #[error("panel {0} is placed twice")]
    PanelPlacedTwice(PanelId),
    /// A panel is in the layout but in no window.
    #[error("panel {0} is in no window")]
    PanelNotPlaced(PanelId),
    /// A tab group or a split has no children.
    #[error("a tab group or split has no children")]
    EmptyContainer,
    /// A tab group's active tab is past its last tab.
    #[error("active tab {active} is out of range for {tabs} tabs")]
    ActiveTabOutOfRange {
        /// The active index.
        active: usize,
        /// The number of tabs.
        tabs: usize,
    },
    /// A split share is zero, negative or not a number.
    #[error("split share {0} must be above zero")]
    BadShare(f32),
    /// A link has no target panels.
    #[error("a link from panel {0} has no targets")]
    LinkWithoutTargets(PanelId),
    /// A link points back at the panel it starts from.
    #[error("panel {0} cannot link to itself")]
    LinkToItself(PanelId),
    /// A link names the same target panel twice.
    #[error("panel {0} is a link target twice")]
    LinkTargetTwice(PanelId),
    /// There is no link at this index.
    #[error("there is no link {0}")]
    UnknownLink(usize),
    /// Pop in was asked for a panel that is in the main window and was
    /// never popped out.
    #[error("panel {0} is not popped out")]
    NotPoppedOut(PanelId),
    /// Pop out was asked for a panel that is already alone in a window.
    #[error("panel {0} is already alone in its own window")]
    AlreadyAlone(PanelId),
    /// The main window cannot be closed while the app runs.
    #[error("the main window cannot be closed")]
    MainWindowStays,
    /// The next panel id is not above every panel id in use.
    #[error("the next panel id {next} must be above the highest id in use, {highest}")]
    NextIdTooLow {
        /// The next panel id in the layout.
        next: PanelId,
        /// The highest panel id in use.
        highest: PanelId,
    },
}
