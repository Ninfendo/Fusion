//! The panel registry and the panel views.

mod ladder;
mod log;
mod placeholder;
mod watchlist;

use std::collections::VecDeque;

use fusion_client_core::demo::DemoMarket;
use fusion_client_core::links::Context;
use fusion_layout::{Gesture, PanelId};
use fusion_theme::Theme;

pub use ladder::LadderView;
pub use log::LogView;
pub use placeholder::Placeholder;
pub use watchlist::{WatchlistView, flash_strength, format_change, gesture_of};

/// A selection a panel sends to the panels linked to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The panel the selection was made in.
    pub from: PanelId,
    /// What was selected.
    pub context: Context,
    /// The modifier key held, if any.
    pub gesture: Option<Gesture>,
}

/// What every panel view gets while it draws.
pub struct PanelContext<'a> {
    /// The panel being drawn.
    pub panel: PanelId,
    /// The colour tokens.
    pub theme: &'a Theme,
    /// The market the panels show. A demo market until the view server exists.
    pub market: &'a DemoMarket,
    /// Seconds on the app's clock.
    pub now: f64,
    /// The app's event log.
    pub events: &'a mut Events,
    /// Selections to send to linked panels after this frame.
    pub outbox: &'a mut Vec<Outgoing>,
}

/// The app's event log: the newest last, at most [`Events::CAPACITY`] lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    lines: VecDeque<String>,
}

impl Events {
    /// The most lines kept.
    pub const CAPACITY: usize = 200;

    /// Adds a line, dropping the oldest when full.
    pub fn push(&mut self, line: impl Into<String>) {
        if self.lines.len() == Self::CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    /// How many lines there are.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// True if there are no lines.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// A panel's view: it draws one panel and keeps that panel's state.
pub trait PanelView {
    /// Draws the panel.
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut PanelContext<'_>);

    /// Takes a selection made in a linked panel. A received context is
    /// never sent on, so links cannot loop.
    fn receive(&mut self, _context: &Context, _events: &mut Events) {}
}

/// A panel type the app can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelKind {
    /// The kind as the layout stores it.
    pub kind: &'static str,
    /// The title a new panel of this kind gets.
    pub title: &'static str,
}

/// Every panel type this build knows, in menu order.
pub const KINDS: [PanelKind; 3] = [
    PanelKind {
        kind: "watchlist",
        title: "Watchlist",
    },
    PanelKind {
        kind: "l2",
        title: "L2",
    },
    PanelKind {
        kind: "log",
        title: "Log",
    },
];

/// A view for a panel kind. A kind this build does not know gets a
/// placeholder, and the panel stays in the layout.
pub fn create(kind: &str) -> Box<dyn PanelView> {
    match kind {
        "watchlist" => Box::new(WatchlistView::default()),
        "l2" => Box::new(LadderView::default()),
        "log" => Box::new(LogView),
        other => Box::new(Placeholder::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_newest_lines() {
        let mut events = Events::default();
        assert!(events.is_empty());
        for n in 0..Events::CAPACITY + 5 {
            events.push(format!("line {n}"));
        }
        assert_eq!(events.len(), Events::CAPACITY);
        assert_eq!(events.lines().next(), Some("line 5"));
        assert_eq!(
            events.lines().next_back(),
            Some(format!("line {}", Events::CAPACITY + 4).as_str())
        );
    }

    #[test]
    fn every_kind_has_a_unique_name() {
        for (i, a) in KINDS.iter().enumerate() {
            for b in &KINDS[i + 1..] {
                assert_ne!(a.kind, b.kind);
            }
        }
    }
}
