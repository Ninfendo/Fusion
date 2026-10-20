//! Cells: one value in a row, as Fusion works with it, and on the wire.

use std::cmp::Ordering;

use fusion_domain::{Decimal, Timestamp};

use crate::v1;

/// One cell of a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// No value.
    Empty,
    /// Text.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// An exact decimal: prices, quantities.
    Decimal(Decimal),
    /// A float, for values that need not be exact, such as a percentage.
    Real(f64),
    /// True or false.
    Flag(bool),
    /// A point in time.
    Time(Timestamp),
}

impl Cell {
    /// The order of kinds when cells of different kinds are compared.
    fn rank(&self) -> u8 {
        match self {
            Self::Empty => 0,
            Self::Flag(_) => 1,
            Self::Integer(_) => 2,
            Self::Decimal(_) => 3,
            Self::Real(_) => 4,
            Self::Time(_) => 5,
            Self::Text(_) => 6,
        }
    }

    /// A total order for sorting: empty cells first, then by kind, then by
    /// value. Numbers of different kinds compare by kind, not by value; a
    /// column holds one kind.
    pub fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Integer(a), Self::Integer(b)) => a.cmp(b),
            (Self::Decimal(a), Self::Decimal(b)) => a.cmp(b),
            (Self::Real(a), Self::Real(b)) => a.total_cmp(b),
            (Self::Flag(a), Self::Flag(b)) => a.cmp(b),
            (Self::Time(a), Self::Time(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

/// A decimal that does not fit the wire: more than 18 significant digits,
/// or more than 28 decimals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecimalOutOfRange;

impl std::fmt::Display for DecimalOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a decimal does not fit the wire format")
    }
}

impl std::error::Error for DecimalOutOfRange {}

impl TryFrom<&Cell> for v1::Value {
    type Error = DecimalOutOfRange;

    fn try_from(cell: &Cell) -> Result<Self, Self::Error> {
        use v1::value::Kind;
        let kind = match cell {
            Cell::Empty => None,
            Cell::Text(t) => Some(Kind::Text(t.clone())),
            Cell::Integer(i) => Some(Kind::Integer(*i)),
            Cell::Decimal(d) => Some(Kind::Decimal(decimal_to_wire(*d)?)),
            Cell::Real(r) => Some(Kind::Real(*r)),
            Cell::Flag(b) => Some(Kind::Flag(*b)),
            Cell::Time(t) => Some(Kind::Time(t.unix_nanos())),
        };
        Ok(Self { kind })
    }
}

impl TryFrom<&v1::Value> for Cell {
    type Error = DecimalOutOfRange;

    fn try_from(value: &v1::Value) -> Result<Self, Self::Error> {
        use v1::value::Kind;
        Ok(match &value.kind {
            None => Self::Empty,
            Some(Kind::Text(t)) => Self::Text(t.clone()),
            Some(Kind::Integer(i)) => Self::Integer(*i),
            Some(Kind::Decimal(d)) => Self::Decimal(decimal_from_wire(d)?),
            Some(Kind::Real(r)) => Self::Real(*r),
            Some(Kind::Flag(b)) => Self::Flag(*b),
            Some(Kind::Time(t)) => Self::Time(Timestamp::from_unix_nanos(*t)),
        })
    }
}

fn decimal_to_wire(value: Decimal) -> Result<v1::Decimal, DecimalOutOfRange> {
    let value = value.normalize();
    Ok(v1::Decimal {
        mantissa: i64::try_from(value.mantissa()).map_err(|_| DecimalOutOfRange)?,
        scale: value.scale(),
    })
}

fn decimal_from_wire(value: &v1::Decimal) -> Result<Decimal, DecimalOutOfRange> {
    Decimal::try_from_i128_with_scale(i128::from(value.mantissa), value.scale)
        .map_err(|_| DecimalOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().expect("a decimal")
    }

    fn round_trip(cell: &Cell) -> Cell {
        let wire = v1::Value::try_from(cell).expect("fits");
        let bytes = crate::encode(&wire);
        let back: v1::Value = crate::decode(&bytes).expect("decodes");
        Cell::try_from(&back).expect("fits")
    }

    #[test]
    fn every_kind_of_cell_survives_the_wire() {
        let cells = [
            Cell::Empty,
            Cell::Text("BTCUSDT".to_owned()),
            Cell::Integer(-42),
            Cell::Decimal(dec("82731.35000000")),
            Cell::Real(-0.25),
            Cell::Flag(true),
            Cell::Time(Timestamp::from_unix_nanos(1_792_514_598_000_000_000)),
        ];
        for cell in &cells {
            assert_eq!(&round_trip(cell), cell);
        }
    }

    #[test]
    fn decimals_go_short_on_the_wire() {
        let wire = v1::Value::try_from(&Cell::Decimal(dec("82731.35000000"))).expect("fits");
        assert_eq!(
            wire.kind,
            Some(v1::value::Kind::Decimal(v1::Decimal {
                mantissa: 8_273_135,
                scale: 2
            }))
        );
    }

    #[test]
    fn decimals_that_do_not_fit_are_refused() {
        let huge = Decimal::from_i128_with_scale(i128::from(i64::MAX) + 1, 0);
        assert_eq!(
            v1::Value::try_from(&Cell::Decimal(huge)),
            Err(DecimalOutOfRange)
        );
        let too_fine = v1::Value {
            kind: Some(v1::value::Kind::Decimal(v1::Decimal {
                mantissa: 1,
                scale: 29,
            })),
        };
        assert_eq!(Cell::try_from(&too_fine), Err(DecimalOutOfRange));
        assert!(!DecimalOutOfRange.to_string().is_empty());
    }

    #[test]
    fn cells_sort_by_value_and_empty_cells_first() {
        let order = |a: &Cell, b: &Cell| a.compare(b);
        assert_eq!(order(&Cell::Integer(1), &Cell::Integer(2)), Ordering::Less);
        assert_eq!(
            order(&Cell::Decimal(dec("2.5")), &Cell::Decimal(dec("2.50"))),
            Ordering::Equal
        );
        assert_eq!(
            order(&Cell::Decimal(dec("100.5")), &Cell::Decimal(dec("99.9"))),
            Ordering::Greater
        );
        assert_eq!(
            order(&Cell::Text("b".into()), &Cell::Text("a".into())),
            Ordering::Greater
        );
        assert_eq!(
            order(&Cell::Real(f64::NAN), &Cell::Real(1.0)),
            Ordering::Greater
        );
        assert_eq!(order(&Cell::Flag(false), &Cell::Flag(true)), Ordering::Less);
        let t = |n| Cell::Time(Timestamp::from_unix_nanos(n));
        assert_eq!(order(&t(1), &t(2)), Ordering::Less);
        assert_eq!(
            order(&Cell::Empty, &Cell::Integer(i64::MIN)),
            Ordering::Less
        );
        let kinds = [
            Cell::Empty,
            Cell::Flag(true),
            Cell::Integer(9),
            Cell::Decimal(dec("1")),
            Cell::Real(0.0),
            t(0),
            Cell::Text(String::new()),
        ];
        for pair in kinds.windows(2) {
            assert_eq!(order(&pair[0], &pair[1]), Ordering::Less, "{pair:?}");
        }
    }
}
