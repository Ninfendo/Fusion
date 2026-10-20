//! Instruments and the order sizes and prices they allow.

use std::fmt;

use crate::ids::InstrumentId;
use crate::num::{Decimal, Price, Quantity, notional};

/// A tradable instrument on one venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    /// Fusion's id.
    pub id: InstrumentId,
    /// The venue's symbol, for example `BTCUSDT`.
    pub symbol: String,
    /// Prices must be a multiple of this. Zero means no constraint.
    pub tick_size: Price,
    /// Quantities must be a multiple of this. Zero means no constraint.
    pub lot_size: Quantity,
    /// The smallest quantity allowed.
    pub min_quantity: Quantity,
    /// The smallest price times quantity allowed, in the quote currency.
    pub min_notional: Decimal,
}

/// Why a price or a quantity does not fit an instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentError {
    /// The price is zero or negative.
    PriceNotPositive(Price),
    /// The price is not a multiple of the tick size.
    PriceOffTick {
        /// The refused price.
        price: Price,
        /// The instrument's tick size.
        tick_size: Price,
    },
    /// The quantity is zero or negative.
    QuantityNotPositive(Quantity),
    /// The quantity is below the minimum.
    QuantityBelowMinimum {
        /// The refused quantity.
        quantity: Quantity,
        /// The instrument's minimum quantity.
        minimum: Quantity,
    },
    /// The quantity is not a multiple of the lot size.
    QuantityOffLot {
        /// The refused quantity.
        quantity: Quantity,
        /// The instrument's lot size.
        lot_size: Quantity,
    },
    /// Price times quantity is below the minimum.
    NotionalBelowMinimum {
        /// Price times quantity.
        notional: Decimal,
        /// The instrument's minimum order value.
        minimum: Decimal,
    },
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PriceNotPositive(p) => write!(f, "price {p} must be above zero"),
            Self::PriceOffTick { price, tick_size } => {
                write!(
                    f,
                    "price {price} is not a multiple of the tick size {tick_size}"
                )
            }
            Self::QuantityNotPositive(q) => write!(f, "quantity {q} must be above zero"),
            Self::QuantityBelowMinimum { quantity, minimum } => {
                write!(f, "quantity {quantity} is below the minimum {minimum}")
            }
            Self::QuantityOffLot { quantity, lot_size } => {
                write!(
                    f,
                    "quantity {quantity} is not a multiple of the lot size {lot_size}"
                )
            }
            Self::NotionalBelowMinimum { notional, minimum } => {
                write!(f, "order value {notional} is below the minimum {minimum}")
            }
        }
    }
}

impl std::error::Error for InstrumentError {}

/// True if `value` is a multiple of `step`, or if `step` is zero.
fn is_multiple(value: Decimal, step: Decimal) -> bool {
    value.checked_rem(step).is_none_or(|rest| rest.is_zero())
}

impl Instrument {
    /// Checks that a limit price is allowed.
    pub fn check_price(&self, price: Price) -> Result<(), InstrumentError> {
        if !price.is_positive() {
            return Err(InstrumentError::PriceNotPositive(price));
        }
        if !is_multiple(price.value(), self.tick_size.value()) {
            return Err(InstrumentError::PriceOffTick {
                price,
                tick_size: self.tick_size,
            });
        }
        Ok(())
    }

    /// Checks that an order quantity is allowed.
    pub fn check_quantity(&self, quantity: Quantity) -> Result<(), InstrumentError> {
        if !quantity.is_positive() {
            return Err(InstrumentError::QuantityNotPositive(quantity));
        }
        if quantity < self.min_quantity {
            return Err(InstrumentError::QuantityBelowMinimum {
                quantity,
                minimum: self.min_quantity,
            });
        }
        if !is_multiple(quantity.value(), self.lot_size.value()) {
            return Err(InstrumentError::QuantityOffLot {
                quantity,
                lot_size: self.lot_size,
            });
        }
        Ok(())
    }

    /// Checks price, quantity and order value together.
    pub fn check_order(&self, price: Price, quantity: Quantity) -> Result<(), InstrumentError> {
        self.check_price(price)?;
        self.check_quantity(quantity)?;
        let value = notional(price, quantity);
        if value < self.min_notional {
            return Err(InstrumentError::NotionalBelowMinimum {
                notional: value,
                minimum: self.min_notional,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn btc_usdt() -> Instrument {
        Instrument {
            id: InstrumentId(1),
            symbol: "BTCUSDT".to_owned(),
            tick_size: "0.01".parse().expect("valid"),
            lot_size: "0.00001".parse().expect("valid"),
            min_quantity: "0.00001".parse().expect("valid"),
            min_notional: "5".parse().expect("valid"),
        }
    }

    fn p(s: &str) -> Price {
        s.parse().expect("valid price")
    }

    fn q(s: &str) -> Quantity {
        s.parse().expect("valid quantity")
    }

    #[test]
    fn accepts_a_valid_order() {
        assert_eq!(btc_usdt().check_order(p("64000.01"), q("0.001")), Ok(()));
    }

    #[test]
    fn rejects_a_price_off_the_tick() {
        assert!(matches!(
            btc_usdt().check_price(p("64000.015")),
            Err(InstrumentError::PriceOffTick { .. })
        ));
    }

    #[test]
    fn rejects_zero_and_negative_prices() {
        assert!(matches!(
            btc_usdt().check_price(p("0")),
            Err(InstrumentError::PriceNotPositive(_))
        ));
        assert!(matches!(
            btc_usdt().check_price(p("-1")),
            Err(InstrumentError::PriceNotPositive(_))
        ));
    }

    #[test]
    fn rejects_quantities_below_minimum_or_off_lot() {
        assert!(matches!(
            btc_usdt().check_quantity(q("0.000001")),
            Err(InstrumentError::QuantityBelowMinimum { .. })
        ));
        assert!(matches!(
            btc_usdt().check_quantity(q("0.000015")),
            Err(InstrumentError::QuantityOffLot { .. })
        ));
        assert!(matches!(
            btc_usdt().check_quantity(q("0")),
            Err(InstrumentError::QuantityNotPositive(_))
        ));
    }

    #[test]
    fn rejects_a_small_order_value() {
        assert!(matches!(
            btc_usdt().check_order(p("64000"), q("0.00001")),
            Err(InstrumentError::NotionalBelowMinimum { .. })
        ));
    }

    #[test]
    fn zero_tick_and_lot_mean_no_constraint() {
        let mut instrument = btc_usdt();
        instrument.tick_size = Price::ZERO;
        instrument.lot_size = Quantity::ZERO;
        assert_eq!(
            instrument.check_order(p("64000.0001"), q("0.0012345")),
            Ok(())
        );
    }

    #[test]
    fn errors_read_as_sentences() {
        let cases = [
            (
                InstrumentError::PriceNotPositive(p("0")),
                "price 0 must be above zero",
            ),
            (
                InstrumentError::PriceOffTick {
                    price: p("1.005"),
                    tick_size: p("0.01"),
                },
                "price 1.005 is not a multiple of the tick size 0.01",
            ),
            (
                InstrumentError::QuantityNotPositive(q("0")),
                "quantity 0 must be above zero",
            ),
            (
                InstrumentError::QuantityBelowMinimum {
                    quantity: q("0.1"),
                    minimum: q("1"),
                },
                "quantity 0.1 is below the minimum 1",
            ),
            (
                InstrumentError::QuantityOffLot {
                    quantity: q("1.5"),
                    lot_size: q("1"),
                },
                "quantity 1.5 is not a multiple of the lot size 1",
            ),
            (
                InstrumentError::NotionalBelowMinimum {
                    notional: "4".parse().expect("valid"),
                    minimum: "5".parse().expect("valid"),
                },
                "order value 4 is below the minimum 5",
            ),
        ];
        for (err, text) in cases {
            assert_eq!(err.to_string(), text);
        }
    }

    #[test]
    fn an_order_exactly_at_the_minimum_value_is_allowed() {
        // 50000 * 0.0001 = 5, the minimum.
        assert_eq!(btc_usdt().check_order(p("50000"), q("0.0001")), Ok(()));
    }
}
