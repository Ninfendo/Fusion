//! The L2 ladder view: the order book of one instrument.

use egui::{RichText, Sense, vec2};
use fusion_client_core::{Level, ladder::LadderModel, links::Context};
use fusion_egui_theme::color;
use fusion_theme::Rgba;

use super::{Events, PanelContext, PanelView};

/// Levels shown on each side.
pub const DEPTH: usize = 10;

/// The ladder: asks above, bids below, with depth bars.
#[derive(Debug, Default)]
pub struct LadderView {
    model: LadderModel,
}

impl LadderView {
    /// The symbol shown, if any.
    pub fn symbol(&self) -> Option<&str> {
        self.model.symbol()
    }
}

impl PanelView for LadderView {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut PanelContext<'_>) {
        let symbols = cx.market.symbols();
        let current = self
            .model
            .symbol()
            .or(symbols.first().copied())
            .unwrap_or_default()
            .to_owned();
        let mut chosen = current.clone();
        egui::ComboBox::new(ui.scope_id().with("ladder-symbol"), "Symbol")
            .selected_text(chosen.as_str())
            .show_ui(ui, |ui| {
                for symbol in &symbols {
                    ui.selectable_value(&mut chosen, (*symbol).to_owned(), *symbol);
                }
            });
        if self.model.symbol() != Some(chosen.as_str()) {
            cx.events.push(format!("L2: showing {chosen}"));
        }
        self.model.show(chosen.as_str());
        if let Some(book) = cx.market.book(&chosen, DEPTH) {
            self.model.apply_book(book);
        }

        let theme = cx.theme;
        egui::Grid::new(ui.scope_id().with("ladder-grid"))
            .num_columns(2)
            .spacing([12.0, 2.0])
            .show(ui, |ui| {
                for level in self.model.book().asks.iter().rev() {
                    self.level_row(ui, level, theme.sell);
                }
                let spread = self
                    .model
                    .spread()
                    .map_or_else(|| "no spread".to_owned(), |s| format!("spread {s}"));
                ui.label(RichText::new(spread).small().color(color(theme.text_muted)));
                ui.end_row();
                for level in &self.model.book().bids {
                    self.level_row(ui, level, theme.buy);
                }
            });
    }

    fn receive(&mut self, context: &Context, events: &mut Events) {
        match context {
            Context::Instrument { ticker } => {
                if self.model.symbol() != Some(ticker.as_str()) {
                    self.model.show(ticker.as_str());
                    events.push(format!("L2: showing {ticker} (linked)"));
                }
            }
        }
    }
}

/// The depth bar: the left part of a level's cell, as wide as the level's
/// share of the largest level.
pub(crate) fn depth_bar(cell: egui::Rect, fraction: f32) -> egui::Rect {
    let mut bar = cell;
    bar.set_width(cell.width() * fraction);
    bar
}

/// Where a level's quantity text starts: 4 points in from the left edge,
/// centred vertically.
pub(crate) fn quantity_anchor(cell: egui::Rect) -> egui::Pos2 {
    cell.left_center() + vec2(4.0, 0.0)
}

impl LadderView {
    fn level_row(&self, ui: &mut egui::Ui, level: &Level, side: Rgba) {
        ui.label(
            RichText::new(level.price.to_string())
                .monospace()
                .color(color(side)),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(140.0, 16.0), Sense::hover());
        let bar = depth_bar(rect, self.model.depth_fraction(level.quantity));
        ui.painter()
            .rect_filled(bar, 2.0, color(side).gamma_multiply(0.3));
        ui.painter().text(
            quantity_anchor(rect),
            egui::Align2::LEFT_CENTER,
            level.quantity.to_string(),
            egui::FontId::monospace(12.0),
            ui.visuals().text_color(),
        );
        ui.end_row();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable as _};
    use fusion_client_core::demo::DemoMarket;
    use fusion_theme::Theme;

    use super::*;
    use crate::panels::Events;

    /// Draws `view` with a fresh market and returns the events it logged.
    fn draw(view: &mut LadderView, steps: usize, click: Option<&[&str]>) -> Events {
        let theme = Theme::dark();
        let market = DemoMarket::new(1);
        let mut events = Events::default();
        {
            let mut harness = Harness::new_ui(|ui| {
                let mut cx = PanelContext {
                    panel: fusion_layout::PanelId(1),
                    outbox: &mut Vec::new(),
                    theme: &theme,
                    market: &market,
                    now: 0.0,
                    events: &mut events,
                };
                view.ui(ui, &mut cx);
            });
            harness.run_steps(steps);
            for label in click.unwrap_or_default() {
                harness.get_by_label(label).click();
                harness.run_steps(2);
            }
        }
        events
    }

    #[test]
    fn the_ladder_opens_on_the_first_symbol_with_a_spread() {
        let mut view = LadderView::default();
        let events = draw(&mut view, 2, None);
        assert_eq!(view.symbol(), Some("BTCUSDT"));
        assert_eq!(events.lines().next(), Some("L2: showing BTCUSDT"));
        assert_eq!(view.model.book().bids.len(), DEPTH);
    }

    #[test]
    fn picking_another_symbol_switches_the_book() {
        let mut view = LadderView::default();
        let events = draw(&mut view, 2, Some(&["Symbol", "ETHUSDT"]));
        assert_eq!(view.symbol(), Some("ETHUSDT"));
        assert!(events.lines().any(|l| l == "L2: showing ETHUSDT"));
    }

    #[test]
    fn the_depth_bar_takes_its_share_of_the_cell() {
        let cell = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), vec2(140.0, 16.0));
        let bar = depth_bar(cell, 0.25);
        assert_eq!(bar.min, cell.min);
        assert!((bar.width() - 35.0).abs() < 1e-4);
        assert!((bar.height() - 16.0).abs() < 1e-4);
    }

    #[test]
    fn the_quantity_sits_just_inside_the_left_edge() {
        let cell = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), vec2(140.0, 16.0));
        assert_eq!(quantity_anchor(cell), egui::pos2(14.0, 28.0));
    }

    #[test]
    fn the_ladder_draws_its_prices() {
        let theme = Theme::dark();
        let market = DemoMarket::new(1);
        let best_bid = market
            .book("BTCUSDT", DEPTH)
            .and_then(|b| b.bids.first().map(|l| l.price.to_string()))
            .expect("the demo market has a BTCUSDT book");
        let mut events = Events::default();
        let mut view = LadderView::default();
        let mut harness = Harness::new_ui(|ui| {
            let mut cx = PanelContext {
                panel: fusion_layout::PanelId(1),
                outbox: &mut Vec::new(),
                theme: &theme,
                market: &market,
                now: 0.0,
                events: &mut events,
            };
            view.ui(ui, &mut cx);
        });
        harness.run_steps(2);
        assert!(
            harness.query_by_label(&best_bid).is_some(),
            "best bid {best_bid}"
        );
    }

    #[test]
    fn an_unknown_symbol_shows_an_empty_ladder() {
        let mut view = LadderView {
            model: LadderModel::showing("NOPE"),
        };
        let events = draw(&mut view, 2, None);
        assert!(view.model.book().bids.is_empty());
        assert_eq!(view.model.spread(), None);
        assert!(events.is_empty(), "the symbol did not change");
    }
}
