//! Routes a selection from one panel to the panels linked to it.
//!
//! Only a user action sends a context. A panel that receives one never sends
//! it on, so links cannot loop.

use std::collections::BTreeMap;

use fusion_layout::{ContextType, Gesture, Layout, LinkMode, PanelId};

/// What a selection is about, shaped like an FDC3 context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// An instrument (FDC3 `fdc3.instrument`).
    Instrument {
        /// The instrument's ticker, for example `BTCUSDT`.
        ticker: String,
    },
}

impl Context {
    /// An instrument context.
    pub fn instrument(ticker: impl Into<String>) -> Self {
        Self::Instrument {
            ticker: ticker.into(),
        }
    }

    /// The FDC3 type of this context.
    pub fn context_type(&self) -> ContextType {
        match self {
            Self::Instrument { .. } => ContextType::instrument(),
        }
    }
}

/// Decides which panels follow a selection, and remembers what each panel
/// received last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkRouter {
    /// For round-robin links: the index of the next target, by link index.
    turns: BTreeMap<usize, usize>,
    /// Panels the user clicked, the most recent last.
    focus: Vec<PanelId>,
    /// The last context each panel received.
    last: BTreeMap<PanelId, Context>,
}

impl LinkRouter {
    /// Records that the user clicked in `panel`.
    pub fn focused(&mut self, panel: PanelId) {
        self.focus.retain(|p| *p != panel);
        self.focus.push(panel);
    }

    /// Forgets a closed panel.
    pub fn forget(&mut self, panel: PanelId) {
        self.focus.retain(|p| *p != panel);
        self.last.remove(&panel);
    }

    /// Starts every round-robin link over from its first target. Call this
    /// when the links change, because turns are kept by link position.
    pub fn links_changed(&mut self) {
        self.turns.clear();
    }

    /// The last context `panel` received, if any.
    pub fn last_context(&self, panel: PanelId) -> Option<&Context> {
        self.last.get(&panel)
    }

    /// The panels that follow `context`, selected in `from` with `gesture`.
    /// Explicit links come first, in their order, then the colour group.
    /// Each panel appears once.
    pub fn route(
        &mut self,
        layout: &Layout,
        from: PanelId,
        context: &Context,
        gesture: Option<Gesture>,
    ) -> Vec<PanelId> {
        let kind = context.context_type();
        let mut targets = Vec::new();
        for (index, link) in layout.links.iter().enumerate() {
            if link.from == from && link.context == kind && link.gesture == gesture {
                targets.extend(self.pick(index, link.mode, &link.to));
            }
        }
        if gesture.is_none()
            && let Some(colour) = layout.groups.get(&from)
        {
            targets.extend(
                layout
                    .groups
                    .iter()
                    .filter(|(panel, c)| *c == colour && **panel != from)
                    .map(|(panel, _)| *panel),
            );
        }
        let mut unique = Vec::with_capacity(targets.len());
        for target in targets {
            if !unique.contains(&target) {
                unique.push(target);
            }
        }
        for target in &unique {
            self.last.insert(*target, context.clone());
        }
        unique
    }

    /// The targets one link picks.
    fn pick(&mut self, index: usize, mode: LinkMode, to: &[PanelId]) -> Vec<PanelId> {
        match mode {
            LinkMode::All => to.to_vec(),
            LinkMode::RoundRobin => {
                let turn = self.turns.entry(index).or_insert(0);
                let target = to.get(*turn % to.len().max(1)).copied();
                *turn = turn.wrapping_add(1);
                target.into_iter().collect()
            }
            LinkMode::LastFocused => self
                .focus
                .iter()
                .rev()
                .find(|p| to.contains(p))
                .or(to.first())
                .copied()
                .into_iter()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
