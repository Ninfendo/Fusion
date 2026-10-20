//! The view for a panel kind this build does not know.

use super::{PanelContext, PanelView};

/// Shows that the panel kind is unknown. The panel stays in the layout, so a
/// newer build can still show it.
#[derive(Debug)]
pub struct Placeholder {
    kind: String,
}

impl Placeholder {
    /// A placeholder for `kind`.
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

impl PanelView for Placeholder {
    fn ui(&mut self, ui: &mut egui::Ui, _cx: &mut PanelContext<'_>) {
        ui.label(format!(
            "Unknown panel kind \"{}\". This build cannot show it; it stays in your layout.",
            self.kind
        ));
    }
}
