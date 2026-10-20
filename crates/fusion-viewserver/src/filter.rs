//! Viewport filters: a CEL condition on a row's columns.

use std::sync::Arc;

use cel::{Context, Program, Value};
use fusion_protocol::Cell;
use rust_decimal::prelude::ToPrimitive as _;

use crate::ViewError;

/// A compiled filter.
pub struct Filter {
    program: Program,
    /// The columns the condition names, and where they are.
    names: Vec<(String, usize)>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Filter")
    }
}

/// A cell as a CEL value. Decimals become floats: close enough to filter
/// on, and the cell itself stays exact. Times are nanoseconds.
fn to_cel(cell: &Cell) -> Value {
    match cell {
        Cell::Empty => Value::Null,
        Cell::Text(t) => Value::String(Arc::new(t.clone())),
        Cell::Integer(i) => Value::Int(*i),
        Cell::Decimal(d) => Value::Float(d.to_f64().unwrap_or(f64::NAN)),
        Cell::Real(r) => Value::Float(*r),
        Cell::Flag(b) => Value::Bool(*b),
        Cell::Time(t) => Value::Int(t.unix_nanos()),
    }
}

impl Filter {
    /// Compiles a condition on a table with `columns`, for example
    /// `symbol.startsWith("BTC")`. Only the columns it names are given to
    /// it, which keeps each run cheap.
    pub fn compile(source: &str, columns: &[String]) -> Result<Self, ViewError> {
        let program = Program::compile(source).map_err(|err| ViewError::Filter(err.to_string()))?;
        let names = program
            .references()
            .variables()
            .into_iter()
            .filter_map(|name| {
                let at = columns.iter().position(|c| c == name)?;
                Some((name.to_owned(), at))
            })
            .collect();
        Ok(Self { program, names })
    }

    /// True if the row passes. A condition that fails to run, or does not
    /// give true or false, lets the row out.
    pub fn passes(&self, context: &mut Context<'_, '_>, cells: &[Cell]) -> bool {
        for (name, at) in &self.names {
            let cell = cells.get(*at).unwrap_or(&Cell::Empty);
            context.add_variable_from_value(name.clone(), to_cel(cell));
        }
        matches!(self.program.execute(context), Ok(Value::Bool(true)))
    }
}

#[cfg(test)]
mod tests {
    use fusion_domain::Timestamp;

    use super::*;

    fn passes(source: &str, cells: &[Cell]) -> bool {
        let columns: Vec<String> = ["symbol", "bid", "count", "change", "live", "at", "note"]
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let filter = Filter::compile(source, &columns).expect("compiles");
        filter.passes(&mut Context::default(), cells)
    }

    fn row() -> Vec<Cell> {
        vec![
            Cell::Text("BTCUSDT".into()),
            Cell::Decimal("82731.35".parse().expect("a price")),
            Cell::Integer(7),
            Cell::Real(-0.5),
            Cell::Flag(true),
            Cell::Time(Timestamp::from_unix_nanos(1_000)),
            Cell::Empty,
        ]
    }

    #[test]
    fn conditions_see_every_kind_of_cell() {
        assert!(passes(r#"symbol.startsWith("BTC")"#, &row()));
        assert!(!passes(r#"symbol == "ETHUSDT""#, &row()));
        assert!(passes("bid > 80000.0 && bid < 90000.0", &row()));
        assert!(passes("count == 7", &row()));
        assert!(passes("change < 0.0", &row()));
        assert!(passes("live", &row()));
        assert!(passes("at == 1000", &row()));
        assert!(passes("note == null", &row()));
    }

    #[test]
    fn a_condition_that_is_not_true_or_false_lets_the_row_out() {
        assert!(!passes("count", &row()), "an integer, not true");
        assert!(
            !passes("missing > 1", &row()),
            "an unknown name fails to run"
        );
    }

    #[test]
    fn a_condition_that_does_not_parse_is_refused() {
        let err = Filter::compile("symbol ==", &[]).expect_err("refused");
        assert!(matches!(err, ViewError::Filter(_)));
        assert_eq!(
            format!("{:?}", Filter::compile("true", &[]).expect("compiles")),
            "Filter"
        );
    }
}
