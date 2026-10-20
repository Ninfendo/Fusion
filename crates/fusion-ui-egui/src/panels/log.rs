//! The log view: what happened in the app, newest at the bottom.

use egui::RichText;
use fusion_egui_theme::color;

use super::{PanelContext, PanelView};

/// The log panel.
#[derive(Debug, Default)]
pub struct LogView;

impl PanelView for LogView {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut PanelContext<'_>) {
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if cx.events.is_empty() {
                    ui.label(RichText::new("No events yet.").color(color(cx.theme.text_muted)));
                }
                for line in cx.events.lines() {
                    ui.label(RichText::new(line).monospace());
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable as _};
    use fusion_client_core::demo::DemoMarket;
    use fusion_theme::Theme;

    use super::*;
    use crate::panels::Events;

    fn shown(events: &mut Events) -> (bool, bool) {
        let theme = Theme::light();
        let market = DemoMarket::new(1);
        let mut harness = Harness::new_ui(|ui| {
            let mut cx = PanelContext {
                panel: fusion_layout::PanelId(1),
                outbox: &mut Vec::new(),
                theme: &theme,
                market: &market,
                now: 0.0,
                events,
            };
            LogView.ui(ui, &mut cx);
        });
        harness.run_steps(2);
        (
            harness.query_by_label("No events yet.").is_some(),
            harness.query_by_label("Layout reset").is_some(),
        )
    }

    #[test]
    fn an_empty_log_says_so() {
        assert_eq!(shown(&mut Events::default()), (true, false));
    }

    #[test]
    fn the_log_shows_its_lines() {
        let mut events = Events::default();
        events.push("Layout reset");
        assert_eq!(shown(&mut events), (false, true));
    }
}
