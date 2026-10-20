//! A viewport: one client's window onto one table.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use fusion_protocol::Cell;
use fusion_protocol::v1::{self, Delta, Row, RowUpdate, ServerMessage, Snapshot, server_message};

use crate::ViewError;
use crate::filter::Filter;
use crate::table::Table;

/// Rows a viewport sends at most.
pub const MAX_ROWS: usize = 1000;

/// The default number of messages a second.
pub const DEFAULT_RATE_HZ: u32 = 20;

/// What was last sent: the keys in display order, their cells, the total.
#[derive(Debug, Clone, Default)]
struct Sent {
    keys: Vec<String>,
    cells: BTreeMap<String, Vec<Cell>>,
    total: u32,
}

/// One client's window onto one table.
#[derive(Debug)]
pub struct Viewport {
    id: u32,
    table: String,
    columns: Vec<usize>,
    names: Vec<String>,
    sort: Vec<(usize, bool)>,
    filter: Option<Filter>,
    offset: usize,
    limit: usize,
    rate_hz: u32,
    seq: u64,
    seen: Option<u64>,
    sent: Option<Sent>,
    /// The filter's answer for each row, and the row version it is for:
    /// a pass runs the filter only on rows changed since.
    passed: BTreeMap<String, (u64, bool)>,
}

fn find(table: &Table, name: &str) -> Result<usize, ViewError> {
    table
        .column(name)
        .ok_or_else(|| ViewError::UnknownColumn(name.to_owned()))
}

impl Viewport {
    /// A viewport onto `table` as `spec` says. Its first message will be a
    /// snapshot.
    pub fn open(id: u32, spec: &v1::ViewportSpec, table: &Table) -> Result<Self, ViewError> {
        let columns = if spec.columns.is_empty() {
            (0..table.columns().len()).collect()
        } else {
            spec.columns
                .iter()
                .map(|c| find(table, c))
                .collect::<Result<Vec<_>, _>>()?
        };
        let sort = spec
            .sort
            .iter()
            .map(|s| Ok((find(table, &s.column)?, s.descending)))
            .collect::<Result<Vec<_>, ViewError>>()?;
        let filter = if spec.filter.trim().is_empty() {
            None
        } else {
            Some(Filter::compile(&spec.filter, table.columns())?)
        };
        let limit = match usize::try_from(spec.limit).unwrap_or(MAX_ROWS) {
            0 => MAX_ROWS,
            n => n.min(MAX_ROWS),
        };
        Ok(Self {
            id,
            table: table.name().to_owned(),
            names: columns
                .iter()
                .map(|i| table.columns()[*i].clone())
                .collect(),
            columns,
            sort,
            filter,
            offset: usize::try_from(spec.offset).unwrap_or(usize::MAX),
            limit,
            rate_hz: match spec.rate_hz {
                0 => DEFAULT_RATE_HZ,
                n => n,
            },
            seq: 0,
            seen: None,
            sent: None,
            passed: BTreeMap::new(),
        })
    }

    /// The viewport's number.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// The table it shows.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// How many messages a second it may send.
    pub fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    /// Forgets what was sent; the next message is a snapshot. For a client
    /// that fell behind.
    pub fn reset(&mut self) {
        self.sent = None;
        self.seen = None;
    }

    /// The message to send now: a snapshot first, then a delta when
    /// something in range changed. `None` when nothing did.
    pub fn poll(&mut self, table: &Table) -> Option<ServerMessage> {
        if self.sent.is_some() && self.seen == Some(table.version()) {
            return None;
        }
        self.seen = Some(table.version());
        let (keys, total) = self.select(table);
        let rows: Vec<(String, Vec<Cell>)> = keys
            .into_iter()
            .map(|key| {
                let cells = self.project(table.row(&key).unwrap_or_default());
                (key, cells)
            })
            .collect();
        let body = match self.sent.take() {
            None => self.snapshot(&rows, total),
            Some(before) => match delta(&before, &rows, total) {
                Some(d) => d,
                None => {
                    self.sent = Some(before);
                    return None;
                }
            },
        };
        self.sent = Some(Sent {
            keys: rows.iter().map(|(k, _)| k.clone()).collect(),
            cells: rows.into_iter().collect(),
            total,
        });
        self.seq += 1;
        Some(ServerMessage {
            body: Some(match body {
                Body::Snapshot(mut s) => {
                    s.viewport = self.id;
                    s.seq = self.seq;
                    server_message::Body::Snapshot(s)
                }
                Body::Delta(mut d) => {
                    d.viewport = self.id;
                    d.seq = self.seq;
                    server_message::Body::Delta(d)
                }
            }),
        })
    }

    /// The keys in range, in display order, and how many rows pass the filter.
    fn select(&mut self, table: &Table) -> (Vec<String>, u32) {
        let mut keep: Vec<(&String, &[Cell])> = Vec::with_capacity(table.len());
        if let Some(filter) = &self.filter {
            // One context for the whole pass; each row overwrites its names.
            let mut context = cel::Context::default();
            for (key, cells, changed) in table.rows() {
                let known = self.passed.get(key).filter(|(at, _)| *at == changed);
                let pass = match known {
                    Some((_, pass)) => *pass,
                    None => {
                        let pass = filter.passes(&mut context, cells);
                        self.passed.insert(key.clone(), (changed, pass));
                        pass
                    }
                };
                if pass {
                    keep.push((key, cells));
                }
            }
            // Forget rows the table no longer has.
            if self.passed.len() != table.len() {
                self.passed.retain(|key, _| table.row(key).is_some());
            }
        } else {
            keep.extend(table.rows().map(|(key, cells, _)| (key, cells)));
        }
        keep.sort_by(|a, b| self.order(a, b));
        let total = u32::try_from(keep.len()).unwrap_or(u32::MAX);
        let keys = keep
            .into_iter()
            .skip(self.offset)
            .take(self.limit)
            .map(|(k, _)| k.clone())
            .collect();
        (keys, total)
    }

    fn order(&self, a: &(&String, &[Cell]), b: &(&String, &[Cell])) -> Ordering {
        for (column, descending) in &self.sort {
            let (x, y) = (&a.1[*column], &b.1[*column]);
            let ordering = if *descending {
                y.compare(x)
            } else {
                x.compare(y)
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        a.0.cmp(b.0)
    }

    fn project(&self, cells: &[Cell]) -> Vec<Cell> {
        self.columns
            .iter()
            .map(|i| cells.get(*i).cloned().unwrap_or(Cell::Empty))
            .collect()
    }

    fn snapshot(&self, rows: &[(String, Vec<Cell>)], total: u32) -> Body {
        Body::Snapshot(Snapshot {
            viewport: 0,
            seq: 0,
            columns: self.names.clone(),
            total_rows: total,
            rows: rows.iter().map(|(k, c)| wire_row(k, c)).collect(),
        })
    }
}

enum Body {
    Snapshot(Snapshot),
    Delta(Delta),
}

/// A cell on the wire. A decimal too big for the wire goes as an empty
/// cell; Fusion's prices and quantities never are.
fn wire(cell: &Cell) -> v1::Value {
    v1::Value::try_from(cell).unwrap_or_default()
}

fn wire_row(key: &str, cells: &[Cell]) -> Row {
    Row {
        key: key.to_owned(),
        cells: cells.iter().map(wire).collect(),
    }
}

/// The changes from `before` to `rows`, or `None` if there are none.
fn delta(before: &Sent, rows: &[(String, Vec<Cell>)], total: u32) -> Option<Body> {
    let mut added = Vec::new();
    let mut updated = Vec::new();
    for (key, cells) in rows {
        match before.cells.get(key) {
            None => added.push(wire_row(key, cells)),
            Some(old) => {
                if let Some(update) = changed_cells(key, old, cells) {
                    updated.push(update);
                }
            }
        }
    }
    let now: BTreeMap<&str, ()> = rows.iter().map(|(k, _)| (k.as_str(), ())).collect();
    let removed: Vec<String> = before
        .keys
        .iter()
        .filter(|k| !now.contains_key(k.as_str()))
        .cloned()
        .collect();
    let order_changed = !before.keys.iter().eq(rows.iter().map(|(k, _)| k));
    let order = if order_changed {
        rows.iter().map(|(k, _)| k.clone()).collect()
    } else {
        Vec::new()
    };
    let nothing = added.is_empty() && updated.is_empty() && !order_changed;
    if nothing && total == before.total {
        return None;
    }
    Some(Body::Delta(Delta {
        viewport: 0,
        seq: 0,
        total_rows: total,
        added,
        updated,
        removed,
        order,
    }))
}

fn changed_cells(key: &str, old: &[Cell], new: &[Cell]) -> Option<RowUpdate> {
    let mut update = RowUpdate {
        key: key.to_owned(),
        columns: Vec::new(),
        values: Vec::new(),
    };
    for (i, (a, b)) in old.iter().zip(new).enumerate() {
        if a != b {
            update.columns.push(u32::try_from(i).unwrap_or(u32::MAX));
            update.values.push(wire(b));
        }
    }
    (!update.columns.is_empty()).then_some(update)
}

#[cfg(test)]
mod tests;
