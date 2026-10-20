//! A table: rows by key, each a list of cells in column order.

use std::collections::BTreeMap;

use fusion_protocol::Cell;

/// One row: its cells, and the table version of its last change.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    cells: Vec<Cell>,
    changed: u64,
}

/// A table the view server keeps, such as `prices`.
#[derive(Debug, Clone)]
pub struct Table {
    name: String,
    columns: Vec<String>,
    rows: BTreeMap<String, Row>,
    version: u64,
}

impl Table {
    /// An empty table with these columns.
    pub fn new(name: impl Into<String>, columns: &[&str]) -> Self {
        Self {
            name: name.into(),
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            rows: BTreeMap::new(),
            version: 0,
        }
    }

    /// The table's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The column names, in order.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The position of a column.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// Counts every change; a viewport compares it to know if anything changed.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True if there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The cells of a row.
    pub fn row(&self, key: &str) -> Option<&[Cell]> {
        self.rows.get(key).map(|r| r.cells.as_slice())
    }

    /// Every row, by key, with the table version of its last change.
    pub fn rows(&self) -> impl Iterator<Item = (&String, &[Cell], u64)> {
        self.rows
            .iter()
            .map(|(k, r)| (k, r.cells.as_slice(), r.changed))
    }

    /// Sets a whole row. Missing cells are empty; extra cells are dropped.
    pub fn upsert(&mut self, key: impl Into<String>, mut cells: Vec<Cell>) {
        cells.resize(self.columns.len(), Cell::Empty);
        let key = key.into();
        if self.row(&key) != Some(cells.as_slice()) {
            self.version += 1;
            let changed = self.version;
            self.rows.insert(key, Row { cells, changed });
        }
    }

    /// Sets one cell of an existing row. Returns false if there is no such
    /// row or column.
    pub fn set(&mut self, key: &str, column: usize, value: Cell) -> bool {
        let Some(row) = self.rows.get_mut(key) else {
            return false;
        };
        let Some(cell) = row.cells.get_mut(column) else {
            return false;
        };
        if *cell != value {
            *cell = value;
            self.version += 1;
            row.changed = self.version;
        }
        true
    }

    /// Removes a row. Returns false if there was none.
    pub fn remove(&mut self, key: &str) -> bool {
        let removed = self.rows.remove(key).is_some();
        if removed {
            self.version += 1;
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_knows_its_columns() {
        let table = Table::new("prices", &["symbol", "bid"]);
        assert_eq!(table.name(), "prices");
        assert_eq!(table.columns(), ["symbol", "bid"]);
        assert_eq!(table.column("bid"), Some(1));
        assert_eq!(table.column("ask"), None);
        assert!(table.is_empty());
    }

    #[test]
    fn rows_are_set_changed_and_removed() {
        let mut table = Table::new("prices", &["symbol", "bid"]);
        table.upsert("BTC", vec![Cell::Text("BTC".into())]);
        table.upsert("ETH", vec![Cell::Text("ETH".into())]);
        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        assert!(table.remove("ETH"));
        assert_eq!(
            table.row("BTC"),
            Some(&[Cell::Text("BTC".into()), Cell::Empty][..])
        );
        assert!(table.set("BTC", 1, Cell::Integer(5)));
        assert!(!table.set("BTC", 9, Cell::Integer(5)), "no such column");
        assert!(!table.set("ETH", 1, Cell::Integer(5)), "no such row");
        assert_eq!(table.rows().count(), 1);
        let (_, _, changed) = table.rows().next().expect("a row");
        assert_eq!(changed, table.version(), "the last change was to this row");
        assert!(table.remove("BTC"));
        assert!(!table.remove("BTC"));
        assert!(table.is_empty());
    }

    #[test]
    fn the_version_counts_real_changes_only() {
        let mut table = Table::new("t", &["a"]);
        let start = table.version();
        table.upsert("k", vec![Cell::Integer(1)]);
        let after_insert = table.version();
        assert!(after_insert > start);
        table.upsert("k", vec![Cell::Integer(1)]);
        table.set("k", 0, Cell::Integer(1));
        assert_eq!(table.version(), after_insert, "same values change nothing");
        table.set("k", 0, Cell::Integer(2));
        assert_eq!(table.version(), after_insert + 1);
        table.upsert("k", vec![Cell::Integer(3), Cell::Integer(4)]);
        assert_eq!(
            table.row("k"),
            Some(&[Cell::Integer(3)][..]),
            "extra cells dropped"
        );
        assert_eq!(table.version(), after_insert + 2);
        table.remove("k");
        assert_eq!(table.version(), after_insert + 3);
        table.remove("k");
        assert_eq!(table.version(), after_insert + 3);
    }
}
