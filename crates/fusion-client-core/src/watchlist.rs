//! The watchlist panel's presentation model.

use fusion_domain::{Decimal, Price};

use crate::market::Quote;

/// Which way a price last moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// No change yet.
    None,
    /// The last price went up.
    Up,
    /// The last price went down.
    Down,
}

/// One row of the watchlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The quote shown.
    pub quote: Quote,
    /// Change since the open, in percent, rounded to 2 decimals.
    pub change_pct: Decimal,
    /// Which way the last price last moved.
    pub last_move: Move,
    /// When it moved, in seconds on the caller's clock. The view fades the
    /// flash colour out from this time.
    pub moved_at: f64,
}

/// What the user can do in the watchlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Click a row.
    Select(String),
}

/// What the watchlist asks the rest of the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Tell linked panels that this instrument is selected.
    InstrumentSelected(String),
}

/// The watchlist's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WatchlistModel {
    rows: Vec<Row>,
    selected: Option<String>,
}

fn change_pct(last: Price, open: Price) -> Decimal {
    if open.is_zero() {
        return Decimal::ZERO;
    }
    ((last.value() - open.value()) / open.value() * Decimal::ONE_HUNDRED).round_dp(2)
}

impl WatchlistModel {
    /// The rows, in the order the quotes came in.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The selected symbol, if any.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Takes new quotes. A row whose last price changed gets a new
    /// [`Move`] and `moved_at = now`.
    pub fn apply_quotes(&mut self, quotes: &[Quote], now: f64) {
        let mut rows = Vec::with_capacity(quotes.len());
        for quote in quotes {
            let previous = self.rows.iter().find(|r| r.quote.symbol == quote.symbol);
            let (last_move, moved_at) = match previous {
                Some(row) if quote.last > row.quote.last => (Move::Up, now),
                Some(row) if quote.last < row.quote.last => (Move::Down, now),
                Some(row) => (row.last_move, row.moved_at),
                None => (Move::None, now),
            };
            rows.push(Row {
                quote: quote.clone(),
                change_pct: change_pct(quote.last, quote.open),
                last_move,
                moved_at,
            });
        }
        self.rows = rows;
    }

    /// Applies a user action and returns what the app must do next.
    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Select(symbol) => {
                // A click on the selected row sends again: with a gesture it
                // may go to another panel.
                self.selected = Some(symbol.clone());
                vec![Effect::InstrumentSelected(symbol)]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(symbol: &str, last: &str, open: &str) -> Quote {
        let p = |s: &str| s.parse::<Price>().expect("valid price");
        Quote {
            symbol: symbol.to_owned(),
            bid: p(last),
            ask: p(last),
            last: p(last),
            open: p(open),
        }
    }

    #[test]
    fn rows_follow_the_quotes() {
        let mut model = WatchlistModel::default();
        model.apply_quotes(&[quote("A", "10", "10"), quote("B", "20", "20")], 1.0);
        let symbols: Vec<&str> = model
            .rows()
            .iter()
            .map(|r| r.quote.symbol.as_str())
            .collect();
        assert_eq!(symbols, ["A", "B"]);
        assert!(model.rows().iter().all(|r| r.last_move == Move::None));
    }

    #[test]
    fn a_price_change_records_its_direction_and_time() {
        let mut model = WatchlistModel::default();
        model.apply_quotes(&[quote("A", "10", "10"), quote("B", "20", "20")], 1.0);
        model.apply_quotes(&[quote("A", "11", "10"), quote("B", "19", "20")], 2.0);
        assert_eq!(model.rows()[0].last_move, Move::Up);
        assert_eq!(model.rows()[1].last_move, Move::Down);
        assert!((model.rows()[0].moved_at - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_unchanged_price_keeps_its_last_move_and_time() {
        let mut model = WatchlistModel::default();
        model.apply_quotes(&[quote("A", "10", "10")], 1.0);
        model.apply_quotes(&[quote("A", "11", "10")], 2.0);
        model.apply_quotes(&[quote("A", "11", "10")], 3.0);
        assert_eq!(model.rows()[0].last_move, Move::Up);
        assert!((model.rows()[0].moved_at - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn change_is_measured_from_the_open() {
        let mut model = WatchlistModel::default();
        model.apply_quotes(&[quote("A", "105", "100"), quote("B", "0.3", "0.4")], 1.0);
        assert_eq!(
            model.rows()[0].change_pct,
            "5".parse::<Decimal>().expect("valid")
        );
        assert_eq!(
            model.rows()[1].change_pct,
            "-25".parse::<Decimal>().expect("valid")
        );
    }

    #[test]
    fn a_zero_open_gives_zero_change() {
        let mut model = WatchlistModel::default();
        model.apply_quotes(&[quote("A", "1", "0")], 1.0);
        assert_eq!(model.rows()[0].change_pct, Decimal::ZERO);
    }

    #[test]
    fn every_click_tells_linked_panels() {
        let mut model = WatchlistModel::default();
        for symbol in ["A", "A", "B"] {
            assert_eq!(
                model.update(Action::Select(symbol.to_owned())),
                [Effect::InstrumentSelected(symbol.to_owned())]
            );
            assert_eq!(model.selected(), Some(symbol));
        }
    }
}
