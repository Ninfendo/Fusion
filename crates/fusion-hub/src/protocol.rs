//! The messages between windows and the hub.

use fusion_client_core::links::Context;
use fusion_layout::{Gesture, Layout, Link, LinkColour, Node, PanelId, WindowId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A context in the FDC3 JSON shape, for example
/// `{"type": "fdc3.instrument", "id": {"ticker": "ETHUSDT"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Fdc3Context {
    /// `fdc3.instrument`.
    #[serde(rename = "fdc3.instrument")]
    Instrument {
        /// The instrument's identifiers.
        id: InstrumentIds,
    },
}

/// The identifiers of an FDC3 instrument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentIds {
    /// The ticker, for example `ETHUSDT`.
    pub ticker: String,
}

impl From<&Context> for Fdc3Context {
    fn from(context: &Context) -> Self {
        match context {
            Context::Instrument { ticker } => Self::Instrument {
                id: InstrumentIds {
                    ticker: ticker.clone(),
                },
            },
        }
    }
}

impl From<Fdc3Context> for Context {
    fn from(context: Fdc3Context) -> Self {
        match context {
            Fdc3Context::Instrument { id } => Self::instrument(id.ticker),
        }
    }
}

/// A message from a window to the hub.
///
/// Messages are externally tagged (`{"select": {...}}`): serde decodes them
/// without buffering, which is faster, and which an internally tagged enum
/// cannot do for the layout's number-keyed maps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToHub {
    /// A window process has started. The hub answers with the layout and
    /// the last context of each of the window's panels.
    Hello {
        /// The window.
        window: WindowId,
    },
    /// The user dragged, resized, switched or closed tabs in a window.
    WindowEdited {
        /// The window.
        window: WindowId,
        /// Its dock tree now.
        root: Option<Node>,
        /// Panels the user closed.
        closed: Vec<PanelId>,
    },
    /// Add a panel to a window.
    AddPanel {
        /// The window.
        window: WindowId,
        /// The panel kind.
        kind: String,
        /// The tab title.
        title: String,
    },
    /// Put a panel in a colour group, or take it out.
    SetGroup {
        /// The panel.
        panel: PanelId,
        /// The group, or `None`.
        colour: Option<LinkColour>,
    },
    /// Add a link.
    AddLink {
        /// The link.
        link: Link,
    },
    /// Remove the link at an index.
    RemoveLink {
        /// The index.
        index: usize,
    },
    /// Replace the whole layout, for example with the default one.
    ReplaceLayout {
        /// The new layout.
        layout: Layout,
    },
    /// Move a panel into a window of its own.
    PopOut {
        /// The panel.
        panel: PanelId,
    },
    /// Move a popped-out panel back.
    PopIn {
        /// The panel.
        panel: PanelId,
    },
    /// The user closed a window. Closing the main window ends the app.
    CloseWindow {
        /// The window.
        window: WindowId,
    },
    /// The user selected something in a panel.
    Select {
        /// The panel.
        from: PanelId,
        /// What was selected.
        context: Fdc3Context,
        /// The modifier key held.
        gesture: Option<Gesture>,
    },
    /// The user clicked in a panel, for last-focused links.
    Focused {
        /// The panel.
        panel: PanelId,
    },
}

/// A message from the hub to a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FromHub {
    /// The layout as it is now.
    Layout {
        /// The layout.
        layout: Layout,
    },
    /// A context for one of the window's panels.
    Deliver {
        /// The panel.
        panel: PanelId,
        /// The context.
        context: Fdc3Context,
    },
    /// Something for the window's log, for example why a change was refused.
    Notice {
        /// The text.
        text: String,
    },
    /// Close the window process.
    Quit,
}

/// A message as one line of JSON, ending in a newline.
pub fn encode<T: Serialize>(message: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// A message from one line of JSON. The newline is optional.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_use_the_fdc3_shape() {
        let context = Fdc3Context::from(&Context::instrument("ETHUSDT"));
        let json = serde_json::to_string(&context).expect("serialises");
        assert_eq!(
            json,
            r#"{"type":"fdc3.instrument","id":{"ticker":"ETHUSDT"}}"#
        );
        let back: Fdc3Context = serde_json::from_str(&json).expect("parses");
        assert_eq!(Context::from(back), Context::instrument("ETHUSDT"));
    }

    #[test]
    fn messages_are_single_json_lines() {
        let message = ToHub::Select {
            from: PanelId(1),
            context: Fdc3Context::from(&Context::instrument("BTCUSDT")),
            gesture: Some(Gesture::Shift),
        };
        let line = encode(&message).expect("encodes");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.starts_with(r#"{"select":{"from":1,"#));
        assert_eq!(decode::<ToHub>(&line).expect("decodes"), message);
    }

    #[test]
    fn a_layout_travels_on_one_line() {
        let mut layout = Layout::new("Fusion");
        layout
            .add_panel(WindowId::MAIN, fusion_layout::Panel::new("log", "Log"))
            .expect("added");
        let line = encode(&FromHub::Layout {
            layout: layout.clone(),
        })
        .expect("encodes");
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(
            decode::<FromHub>(&line).expect("decodes"),
            FromHub::Layout { layout }
        );
    }

    #[test]
    fn a_bad_line_is_an_error() {
        assert!(decode::<ToHub>("{ nope").is_err());
        assert!(decode::<ToHub>(r#"{"unknown":{}}"#).is_err());
    }

    #[test]
    fn simple_messages_read_well() {
        assert_eq!(encode(&FromHub::Quit).expect("encodes"), "\"quit\"\n");
        assert_eq!(
            encode(&ToHub::Hello {
                window: WindowId(2)
            })
            .expect("encodes"),
            "{\"hello\":{\"window\":2}}\n"
        );
    }
}
