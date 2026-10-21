//! The watchlist view.

use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder, TableRow};
use fusion_client_core::links::Context;
use fusion_client_core::watchlist::{Action, Effect, Move, Row, WatchlistModel};
use fusion_egui_theme::color;
use fusion_layout::Gesture;
use fusion_theme::{Rgba, Theme};

use super::{Outgoing, PanelContext, PanelView};

/// The link gesture for the keys held during a click: Shift wins over Ctrl
/// (or Cmd), which wins over Alt.
pub fn gesture_of(keys: egui::Modifiers) -> Option<Gesture> {
    if keys.shift {
        Some(Gesture::Shift)
    } else if keys.command || keys.ctrl {
        Some(Gesture::Ctrl)
    } else if keys.alt {
        Some(Gesture::Alt)
    } else {
        None
    }
}

/// How long a price flash takes to fade, in seconds.
pub const FLASH_SECONDS: f64 = 0.8;

/// How strong a flash is `age` seconds after the move: 1 at once, 0 when
/// it has faded.
pub fn flash_strength(age: f64) -> f32 {
    (1.0 - age / FLASH_SECONDS).clamp(0.0, 1.0) as f32
}

/// A change in percent with its sign: `+1.25`, `-0.40`, `0.00`.
pub fn format_change(change: fusion_client_core::Decimal) -> String {
    if change.is_sign_positive() && !change.is_zero() {
        format!("+{change:.2}")
    } else {
        format!("{change:.2}")
    }
}

/// The watchlist: one row per instrument, prices flash on change, and a
/// click selects the instrument.
#[derive(Debug, Default)]
pub struct WatchlistView {
    model: WatchlistModel,
}

impl PanelView for WatchlistView {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut PanelContext<'_>) {
        self.model.apply_quotes(cx.market.quotes(), cx.now);
        let (theme, now) = (cx.theme, cx.now);
        let clicked = rows_table(ui, &self.model, theme, now);
        let gesture = ui.input(|i| gesture_of(i.modifiers));
        let effects = clicked.map(|s| self.model.update(Action::Select(s)));
        for effect in effects.into_iter().flatten() {
            match effect {
                Effect::InstrumentSelected(s) => {
                    cx.events.push(format!("Watchlist: {s} selected"));
                    cx.outbox.push(Outgoing {
                        from: cx.panel,
                        context: Context::instrument(s),
                        gesture,
                    });
                }
            }
        }
    }
}

/// The height of a row, in points.
pub const ROW_HEIGHT: f32 = 20.0;

/// The column titles.
const TITLES: [&str; 5] = ["Symbol", "Bid", "Ask", "Last", "Chg %"];

/// The table of rows, with its header fixed on top. Only the rows on
/// screen are laid out, so 500 instruments cost about what 40 do. Returns
/// the symbol clicked, if any.
fn rows_table(
    ui: &mut egui::Ui,
    model: &WatchlistModel,
    theme: &Theme,
    now: f64,
) -> Option<String> {
    let mut clicked = None;
    let rows = model.rows();
    TableBuilder::new(ui)
        .id_salt("watchlist-table")
        .striped(true)
        .columns(Column::auto().at_least(80.0), 4)
        .column(Column::remainder().at_least(50.0))
        .header(ROW_HEIGHT, |mut header| {
            for title in TITLES {
                header.col(|ui| {
                    ui.label(RichText::new(title).strong().color(color(theme.text_muted)));
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, rows.len(), |mut cells| {
                let Some(row) = rows.get(cells.index()) else {
                    return;
                };
                if draw_row(&mut cells, row, is_selected(model, row), theme, now) {
                    clicked = Some(row.quote.symbol.clone());
                }
            });
        });
    clicked
}

/// True if the row is the selected instrument.
fn is_selected(model: &WatchlistModel, row: &Row) -> bool {
    model.selected() == Some(row.quote.symbol.as_str())
}

/// The fill behind a row's last price at time `now`: the up or down tick
/// colour, fading out after the move.
fn row_flash(row: &Row, theme: &Theme, now: f64) -> Color32 {
    let flash = match row.last_move {
        Move::Up => color(theme.up_tick),
        Move::Down => color(theme.down_tick),
        Move::None => Color32::TRANSPARENT,
    };
    flash.gamma_multiply(flash_strength(now - row.moved_at))
}

/// The colour a change is shown in: buy colour up, sell colour down.
fn change_colour(change: fusion_client_core::Decimal, theme: &Theme) -> Rgba {
    if change.is_zero() {
        theme.text_muted
    } else if change.is_sign_negative() {
        theme.sell
    } else {
        theme.buy
    }
}

/// Draws one row and returns true if its symbol was clicked.
fn draw_row(
    cells: &mut TableRow<'_, '_>,
    row: &Row,
    selected: bool,
    theme: &Theme,
    now: f64,
) -> bool {
    let mut clicked = false;
    cells.col(|ui| {
        clicked = ui
            .selectable_label(selected, row.quote.symbol.as_str())
            .clicked();
    });
    let price = |text: String, c: Rgba| RichText::new(text).monospace().color(color(c));
    cells.col(|ui| {
        ui.label(price(row.quote.bid.to_string(), theme.buy));
    });
    cells.col(|ui| {
        ui.label(price(row.quote.ask.to_string(), theme.sell));
    });
    cells.col(|ui| {
        egui::Frame::new()
            .fill(row_flash(row, theme, now))
            .show(ui, |ui| {
                ui.label(price(row.quote.last.to_string(), theme.text))
            });
    });
    let change = row.change_pct;
    cells.col(|ui| {
        ui.label(price(format_change(change), change_colour(change, theme)));
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_fades_over_its_time() {
        assert!((flash_strength(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((flash_strength(FLASH_SECONDS / 2.0) - 0.5).abs() < 1e-6);
        assert!(flash_strength(FLASH_SECONDS).abs() < f32::EPSILON);
        assert!(flash_strength(10.0).abs() < f32::EPSILON);
        assert!((flash_strength(-1.0) - 1.0).abs() < f32::EPSILON);
    }

    use fusion_client_core::Quote;

    fn row(last_move: Move, moved_at: f64) -> Row {
        let p = |s: &str| s.parse().expect("valid price");
        Row {
            quote: Quote {
                symbol: "ETHUSDT".to_owned(),
                bid: p("1"),
                ask: p("2"),
                last: p("1"),
                open: p("1"),
            },
            change_pct: fusion_client_core::Decimal::ZERO,
            last_move,
            moved_at,
        }
    }

    #[test]
    fn a_fresh_move_flashes_in_full_and_then_fades() {
        let theme = Theme::dark();
        let up = row(Move::Up, 10.0);
        assert_eq!(row_flash(&up, &theme, 10.0), color(theme.up_tick));
        assert_eq!(
            row_flash(&up, &theme, 10.0 + FLASH_SECONDS / 2.0),
            color(theme.up_tick).gamma_multiply(0.5)
        );
        assert_eq!(
            row_flash(&up, &theme, 10.0 + FLASH_SECONDS),
            Color32::TRANSPARENT
        );
        let down = row(Move::Down, 10.0);
        assert_eq!(row_flash(&down, &theme, 10.0), color(theme.down_tick));
        assert_eq!(
            row_flash(&row(Move::None, 10.0), &theme, 10.0),
            Color32::TRANSPARENT
        );
    }

    #[test]
    fn only_the_selected_row_is_selected() {
        let mut model = WatchlistModel::default();
        let eth = row(Move::None, 0.0);
        assert!(!is_selected(&model, &eth));
        model.update(Action::Select("ETHUSDT".to_owned()));
        assert!(is_selected(&model, &eth));
        model.update(Action::Select("BTCUSDT".to_owned()));
        assert!(!is_selected(&model, &eth));
    }

    #[test]
    fn changes_are_coloured_by_direction() {
        let theme = Theme::dark();
        let d = |s: &str| s.parse().expect("valid decimal");
        assert_eq!(change_colour(d("1"), &theme), theme.buy);
        assert_eq!(change_colour(d("-1"), &theme), theme.sell);
        assert_eq!(change_colour(d("0"), &theme), theme.text_muted);
    }

    #[test]
    fn keys_map_to_gestures() {
        use egui::Modifiers;
        assert_eq!(gesture_of(Modifiers::NONE), None);
        assert_eq!(gesture_of(Modifiers::SHIFT), Some(Gesture::Shift));
        assert_eq!(gesture_of(Modifiers::CTRL), Some(Gesture::Ctrl));
        assert_eq!(gesture_of(Modifiers::COMMAND), Some(Gesture::Ctrl));
        assert_eq!(gesture_of(Modifiers::ALT), Some(Gesture::Alt));
        assert_eq!(
            gesture_of(Modifiers::SHIFT | Modifiers::CTRL | Modifiers::ALT),
            Some(Gesture::Shift)
        );
        assert_eq!(
            gesture_of(Modifiers::CTRL | Modifiers::ALT),
            Some(Gesture::Ctrl)
        );
    }

    #[test]
    fn changes_show_their_sign() {
        let d = |s: &str| s.parse().expect("valid decimal");
        assert_eq!(format_change(d("1.25")), "+1.25");
        assert_eq!(format_change(d("-0.4")), "-0.40");
        assert_eq!(format_change(d("0")), "0.00");
    }
}
