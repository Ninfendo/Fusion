//! Links between panels: which panels follow a selection made in another.

use serde::{Deserialize, Serialize};

use crate::PanelId;

/// A context type, by its FDC3 name, for example `fdc3.instrument`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContextType(pub String);

impl ContextType {
    /// The FDC3 name of an instrument context.
    pub const INSTRUMENT: &'static str = "fdc3.instrument";

    /// The instrument context type.
    pub fn instrument() -> Self {
        Self(Self::INSTRUMENT.to_owned())
    }
}

/// Which of a link's target panels follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkMode {
    /// Every target follows.
    #[default]
    All,
    /// The next target in turn follows.
    RoundRobin,
    /// The target the user clicked last follows.
    LastFocused,
}

/// A modifier key held during the click. A link with a gesture only fires
/// for that gesture; a link without one fires for a plain click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gesture {
    /// Shift+click.
    Shift,
    /// Ctrl+click (Cmd+click on macOS).
    Ctrl,
    /// Alt+click.
    Alt,
}

/// One link: a context selected in `from` goes to the `to` panels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The panel the selection is made in.
    pub from: PanelId,
    /// The kind of context that travels.
    pub context: ContextType,
    /// The panels that may follow.
    pub to: Vec<PanelId>,
    /// Which of them follow.
    #[serde(default)]
    pub mode: LinkMode,
    /// The modifier key this link needs. `None` for a plain click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<Gesture>,
}

/// A colour group. Every panel in a group follows the others on a plain
/// click, for every context type: a shortcut for links between all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkColour {
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Yellow.
    Yellow,
}

impl LinkColour {
    /// Every colour, in menu order.
    pub const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Blue, Self::Yellow];

    /// The colour's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Yellow => "Yellow",
        }
    }
}
