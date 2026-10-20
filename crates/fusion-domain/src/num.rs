//! Prices and quantities: exact decimals, never floats.

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

pub use rust_decimal::Decimal;

macro_rules! decimal_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(Decimal);

        impl $name {
            /// Zero.
            pub const ZERO: Self = Self(Decimal::ZERO);

            /// Wraps an exact decimal.
            pub const fn new(value: Decimal) -> Self {
                Self(value)
            }

            /// The exact decimal value.
            pub const fn value(self) -> Decimal {
                self.0
            }

            /// True if the value is zero.
            pub fn is_zero(self) -> bool {
                self.0.is_zero()
            }

            /// True if the value is greater than zero.
            pub fn is_positive(self) -> bool {
                self.0 > Decimal::ZERO
            }
        }

        impl FromStr for $name {
            type Err = rust_decimal::Error;

            /// Parses an exact decimal such as `"0.01"`. Rejects values that
            /// would lose precision.
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Decimal::from_str_exact(s).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

decimal_newtype!(
    /// A price per unit of an instrument.
    Price
);

decimal_newtype!(
    /// An amount of an instrument.
    Quantity
);

impl Add for Quantity {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Quantity {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// Price times quantity, in the quote currency.
pub fn notional(price: Price, quantity: Quantity) -> Decimal {
    price.0 * quantity.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_decimals() {
        let p: Price = "0.01".parse().expect("valid price");
        assert_eq!(p.to_string(), "0.01");
        let q: Quantity = "1.5".parse().expect("valid quantity");
        assert_eq!(q.value(), Decimal::new(15, 1));
    }

    #[test]
    fn decimal_maths_is_exact() {
        let a: Quantity = "0.1".parse().expect("valid");
        let b: Quantity = "0.2".parse().expect("valid");
        assert_eq!(a + b, "0.3".parse().expect("valid"));
        assert_eq!((a + b) - a, b);
    }

    #[test]
    fn notional_is_price_times_quantity() {
        let p: Price = "64000.5".parse().expect("valid");
        let q: Quantity = "0.002".parse().expect("valid");
        assert_eq!(notional(p, q), "128.001".parse::<Decimal>().expect("valid"));
    }

    #[test]
    fn sign_checks() {
        assert!(Quantity::ZERO.is_zero());
        assert!(!Quantity::ZERO.is_positive());
        assert!("-1".parse::<Price>().expect("valid").value() < Decimal::ZERO);
        assert!("2".parse::<Price>().expect("valid").is_positive());
    }

    #[test]
    fn wraps_a_decimal() {
        let price = Price::new(Decimal::new(12_345, 2));
        assert_eq!(price.to_string(), "123.45");
        assert_eq!(price.value(), Decimal::new(12_345, 2));
    }
}
