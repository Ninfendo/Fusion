//! A client's copy of one viewport, kept up to date from the server's
//! snapshots and deltas.

use std::collections::BTreeMap;

use fusion_protocol::Cell;
use fusion_protocol::v1::{self, Delta, Snapshot};

/// A delta that does not follow the last message: ask for a new snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missed {
    /// The number expected.
    pub expected: u64,
    /// The number that came.
    pub got: u64,
}

/// The rows of one viewport, in display order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewportCache {
    columns: Vec<String>,
    rows: BTreeMap<String, Vec<Cell>>,
    order: Vec<String>,
    total: u32,
    seq: u64,
}

fn cell(value: &v1::Value) -> Cell {
    // A value this build cannot read shows as empty.
    Cell::try_from(value).unwrap_or(Cell::Empty)
}

impl ViewportCache {
    /// Replaces everything with a snapshot.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        self.columns = snapshot.columns;
        self.order = snapshot.rows.iter().map(|r| r.key.clone()).collect();
        self.rows = snapshot
            .rows
            .into_iter()
            .map(|r| (r.key, r.cells.iter().map(cell).collect()))
            .collect();
        self.total = snapshot.total_rows;
        self.seq = snapshot.seq;
    }

    /// Applies a delta that follows the last message. One that does not is
    /// refused, and the copy is left as it was.
    pub fn apply_delta(&mut self, delta: Delta) -> Result<(), Missed> {
        let expected = self.seq + 1;
        if delta.seq != expected {
            return Err(Missed {
                expected,
                got: delta.seq,
            });
        }
        for key in &delta.removed {
            self.rows.remove(key);
        }
        for row in delta.added {
            self.rows
                .insert(row.key, row.cells.iter().map(cell).collect());
        }
        for update in delta.updated {
            let Some(cells) = self.rows.get_mut(&update.key) else {
                continue;
            };
            for (column, value) in update.columns.iter().zip(&update.values) {
                if let Some(slot) = usize::try_from(*column).ok().and_then(|i| cells.get_mut(i)) {
                    *slot = cell(value);
                }
            }
        }
        if !delta.order.is_empty() {
            self.order = delta.order;
        }
        self.total = delta.total_rows;
        self.seq = delta.seq;
        Ok(())
    }

    /// The columns, in order.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The position of a column.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// The rows in display order.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &[Cell])> {
        self.order
            .iter()
            .filter_map(|key| Some((key.as_str(), self.rows.get(key)?.as_slice())))
    }

    /// One row by key.
    pub fn row(&self, key: &str) -> Option<&[Cell]> {
        self.rows.get(key).map(Vec::as_slice)
    }

    /// Rows that pass the filter, in and out of range.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The number of the last message applied.
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

#[cfg(test)]
mod tests {
    use fusion_protocol::v1::{Row, RowUpdate};

    use super::*;

    fn value(cell: &Cell) -> v1::Value {
        v1::Value::try_from(cell).expect("fits")
    }

    fn row(key: &str, n: i64) -> Row {
        Row {
            key: key.to_owned(),
            cells: vec![value(&Cell::Text(key.into())), value(&Cell::Integer(n))],
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            viewport: 1,
            seq: 1,
            columns: vec!["symbol".into(), "n".into()],
            total_rows: 5,
            rows: vec![row("A", 1), row("B", 2)],
        }
    }

    fn keys(cache: &ViewportCache) -> Vec<&str> {
        cache.rows().map(|(k, _)| k).collect()
    }

    #[test]
    fn a_snapshot_fills_the_copy() {
        let mut cache = ViewportCache::default();
        cache.apply_snapshot(snapshot());
        assert_eq!(cache.columns(), ["symbol", "n"]);
        assert_eq!(cache.column("n"), Some(1));
        assert_eq!(cache.column("x"), None);
        assert_eq!(keys(&cache), ["A", "B"]);
        assert_eq!(
            cache.row("B"),
            Some(&[Cell::Text("B".into()), Cell::Integer(2)][..])
        );
        assert_eq!((cache.total(), cache.seq()), (5, 1));
    }

    #[test]
    fn a_delta_adds_updates_removes_and_reorders() {
        let mut cache = ViewportCache::default();
        cache.apply_snapshot(snapshot());
        let delta = Delta {
            viewport: 1,
            seq: 2,
            total_rows: 6,
            added: vec![row("C", 3)],
            updated: vec![RowUpdate {
                key: "B".into(),
                columns: vec![1, 9],
                values: vec![value(&Cell::Integer(20)), value(&Cell::Integer(0))],
            }],
            removed: vec!["A".into()],
            order: vec!["C".into(), "B".into()],
        };
        assert_eq!(cache.apply_delta(delta), Ok(()));
        assert_eq!(keys(&cache), ["C", "B"]);
        assert_eq!(
            cache.row("B").map(|r| r[1].clone()),
            Some(Cell::Integer(20))
        );
        assert_eq!(cache.row("A"), None);
        assert_eq!((cache.total(), cache.seq()), (6, 2));
    }

    #[test]
    fn a_delta_without_a_new_order_keeps_the_order() {
        let mut cache = ViewportCache::default();
        cache.apply_snapshot(snapshot());
        let delta = Delta {
            viewport: 1,
            seq: 2,
            total_rows: 5,
            updated: vec![RowUpdate {
                key: "Z".into(),
                columns: vec![1],
                values: vec![value(&Cell::Integer(9))],
            }],
            ..Delta::default()
        };
        assert_eq!(cache.apply_delta(delta), Ok(()));
        assert_eq!(
            keys(&cache),
            ["A", "B"],
            "an update to an unknown row is ignored"
        );
    }

    #[test]
    fn a_delta_out_of_turn_is_refused() {
        let mut cache = ViewportCache::default();
        cache.apply_snapshot(snapshot());
        let before = cache.clone();
        let late = Delta {
            seq: 3,
            removed: vec!["A".into()],
            ..Delta::default()
        };
        assert_eq!(
            cache.apply_delta(late),
            Err(Missed {
                expected: 2,
                got: 3
            })
        );
        assert_eq!(cache, before);
    }

    #[test]
    fn a_value_this_build_cannot_read_is_empty() {
        let bad = v1::Value {
            kind: Some(v1::value::Kind::Decimal(v1::Decimal {
                mantissa: 1,
                scale: 99,
            })),
        };
        assert_eq!(cell(&bad), Cell::Empty);
    }
}
