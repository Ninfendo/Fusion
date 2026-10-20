//! The L2 ladder panel's presentation model: the order book of one
//! instrument.

use fusion_domain::{Decimal, Price};

use crate::market::Book;

/// The ladder's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LadderModel {
    symbol: Option<String>,
    book: Book,
}

impl LadderModel {
    /// A ladder showing `symbol`.
    pub fn showing(symbol: impl Into<String>) -> Self {
        Self {
            symbol: Some(symbol.into()),
            book: Book::default(),
        }
    }

    /// The instrument shown, if any.
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    /// Switches to another instrument. The old book is dropped at once, so
    /// the ladder never shows one symbol's book under another's name.
    pub fn show(&mut self, symbol: impl Into<String>) {
        let symbol = symbol.into();
        if self.symbol.as_deref() != Some(symbol.as_str()) {
            self.symbol = Some(symbol);
            self.book = Book::default();
        }
    }

    /// Takes a new book for the instrument shown.
    pub fn apply_book(&mut self, book: Book) {
        self.book = book;
    }

    /// The current book.
    pub fn book(&self) -> &Book {
        &self.book
    }

    /// The best ask minus the best bid, if both sides have a level.
    pub fn spread(&self) -> Option<Decimal> {
        let bid: Price = self.book.bids.first()?.price;
        let ask: Price = self.book.asks.first()?.price;
        Some(ask.value() - bid.value())
    }

    /// A level's quantity as a share of the largest on the book, from 0 to
    /// 1, for drawing depth bars.
    pub fn depth_fraction(&self, quantity: fusion_domain::Quantity) -> f32 {
        let max = self.book.max_quantity();
        if max.is_zero() {
            return 0.0;
        }
        let fraction = quantity.value() / max.value();
        f32::try_from(fraction).unwrap_or(0.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use fusion_domain::Quantity;

    use super::*;
    use crate::market::Level;

    fn level(price: &str, quantity: &str) -> Level {
        Level {
            price: price.parse().expect("valid price"),
            quantity: quantity.parse().expect("valid quantity"),
        }
    }

    fn book() -> Book {
        Book {
            bids: vec![level("99.5", "2"), level("99", "8")],
            asks: vec![level("100", "4"), level("100.5", "1")],
        }
    }

    #[test]
    fn a_new_ladder_shows_nothing() {
        let model = LadderModel::default();
        assert_eq!(model.symbol(), None);
        assert_eq!(model.spread(), None);
        assert_eq!(model.book().max_quantity(), Quantity::ZERO);
    }

    #[test]
    fn spread_is_best_ask_minus_best_bid() {
        let mut model = LadderModel::showing("A");
        model.apply_book(book());
        assert_eq!(model.spread(), Some("0.5".parse().expect("valid")));
    }

    #[test]
    fn switching_instrument_drops_the_old_book() {
        let mut model = LadderModel::showing("A");
        model.apply_book(book());
        model.show("B");
        assert_eq!(model.symbol(), Some("B"));
        assert_eq!(model.book(), &Book::default());
    }

    #[test]
    fn showing_the_same_instrument_keeps_the_book() {
        let mut model = LadderModel::showing("A");
        model.apply_book(book());
        model.show("A");
        assert_eq!(model.book(), &book());
    }

    #[test]
    fn depth_bars_scale_to_the_largest_level() {
        let mut model = LadderModel::showing("A");
        model.apply_book(book());
        assert!((model.depth_fraction(Quantity::new(Decimal::new(8, 0))) - 1.0).abs() < 1e-6);
        assert!((model.depth_fraction(Quantity::new(Decimal::new(2, 0))) - 0.25).abs() < 1e-6);
        assert!((model.depth_fraction(Quantity::new(Decimal::new(16, 0))) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_empty_book_has_no_depth() {
        let model = LadderModel::showing("A");
        assert!(model.depth_fraction(Quantity::new(Decimal::ONE)).abs() < f32::EPSILON);
    }
}
