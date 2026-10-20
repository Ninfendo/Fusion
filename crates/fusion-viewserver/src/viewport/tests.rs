use fusion_protocol::v1::{Sort, ViewportSpec};

use super::*;

fn dec(text: &str) -> Cell {
    Cell::Decimal(text.parse().expect("a decimal"))
}

/// symbol, bid, volume: BTC 100 30, ETH 50 20, SOL 10 10.
fn prices() -> Table {
    let mut table = Table::new("prices", &["symbol", "bid", "volume"]);
    for (symbol, bid, volume) in [("BTC", "100", 30), ("ETH", "50", 20), ("SOL", "10", 10)] {
        table.upsert(
            symbol,
            vec![Cell::Text(symbol.into()), dec(bid), Cell::Integer(volume)],
        );
    }
    table
}

fn spec() -> ViewportSpec {
    ViewportSpec {
        table: "prices".to_owned(),
        ..ViewportSpec::default()
    }
}

fn by_volume(descending: bool) -> Vec<Sort> {
    vec![Sort {
        column: "volume".to_owned(),
        descending,
    }]
}

fn snapshot(message: Option<ServerMessage>) -> Snapshot {
    match message.and_then(|m| m.body) {
        Some(server_message::Body::Snapshot(s)) => s,
        other => panic!("not a snapshot: {other:?}"),
    }
}

fn delta(message: Option<ServerMessage>) -> Delta {
    match message.and_then(|m| m.body) {
        Some(server_message::Body::Delta(d)) => d,
        other => panic!("not a delta: {other:?}"),
    }
}

fn keys(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.key.as_str()).collect()
}

#[test]
fn the_first_message_is_a_snapshot_of_every_row() {
    let table = prices();
    let mut view = Viewport::open(3, &spec(), &table).expect("opens");
    assert_eq!(
        (view.id(), view.table(), view.rate_hz()),
        (3, "prices", DEFAULT_RATE_HZ)
    );
    let first = snapshot(view.poll(&table));
    assert_eq!((first.viewport, first.seq, first.total_rows), (3, 1, 3));
    assert_eq!(first.columns, ["symbol", "bid", "volume"]);
    assert_eq!(keys(&first.rows), ["BTC", "ETH", "SOL"], "by key");
    assert_eq!(
        first.rows[0].cells[1],
        v1::Value::try_from(&dec("100")).expect("fits")
    );
    assert!(view.poll(&table).is_none(), "nothing changed");
}

#[test]
fn columns_sort_and_range_shape_the_rows() {
    let table = prices();
    let spec = ViewportSpec {
        columns: vec!["symbol".to_owned(), "volume".to_owned()],
        sort: by_volume(false),
        offset: 1,
        limit: 1,
        rate_hz: 5,
        ..spec()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    assert_eq!(view.rate_hz(), 5);
    let first = snapshot(view.poll(&table));
    assert_eq!(first.columns, ["symbol", "volume"]);
    assert_eq!(keys(&first.rows), ["ETH"], "second smallest volume");
    assert_eq!(first.total_rows, 3);
    assert_eq!(first.rows[0].cells.len(), 2);

    let descending = ViewportSpec {
        sort: by_volume(true),
        ..ViewportSpec::default()
    };
    let mut view = Viewport::open(
        2,
        &ViewportSpec {
            table: "prices".into(),
            ..descending
        },
        &table,
    )
    .expect("opens");
    assert_eq!(
        keys(&snapshot(view.poll(&table)).rows),
        ["BTC", "ETH", "SOL"]
    );
}

#[test]
fn ties_in_the_sort_fall_back_to_the_key() {
    let mut table = Table::new("t", &["n"]);
    for key in ["c", "a", "b"] {
        table.upsert(key, vec![Cell::Integer(1)]);
    }
    let spec = ViewportSpec {
        table: "t".into(),
        sort: vec![Sort {
            column: "n".into(),
            descending: true,
        }],
        ..ViewportSpec::default()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    assert_eq!(keys(&snapshot(view.poll(&table)).rows), ["a", "b", "c"]);
}

#[test]
fn a_filter_keeps_matching_rows_and_counts_them() {
    let table = prices();
    let spec = ViewportSpec {
        filter: "volume >= 20".to_owned(),
        ..spec()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    let first = snapshot(view.poll(&table));
    assert_eq!(keys(&first.rows), ["BTC", "ETH"]);
    assert_eq!(first.total_rows, 2);
}

#[test]
fn a_change_sends_only_the_changed_cells() {
    let mut table = prices();
    let mut view = Viewport::open(1, &spec(), &table).expect("opens");
    view.poll(&table);
    table.set("ETH", 1, dec("51"));
    let change = delta(view.poll(&table));
    assert_eq!(change.seq, 2);
    assert!(change.added.is_empty() && change.removed.is_empty() && change.order.is_empty());
    assert_eq!(change.updated.len(), 1);
    assert_eq!(change.updated[0].key, "ETH");
    assert_eq!(change.updated[0].columns, [1]);
    assert_eq!(
        change.updated[0].values,
        [v1::Value::try_from(&dec("51")).expect("fits")]
    );
}

#[test]
fn a_change_outside_the_range_sends_nothing() {
    let mut table = prices();
    let spec = ViewportSpec { limit: 1, ..spec() };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    view.poll(&table);
    table.set("SOL", 1, dec("11"));
    assert!(view.poll(&table).is_none(), "SOL is out of range");
}

#[test]
fn rows_move_in_and_out_of_range_and_change_order() {
    let mut table = prices();
    let spec = ViewportSpec {
        sort: by_volume(true),
        limit: 2,
        ..spec()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    assert_eq!(keys(&snapshot(view.poll(&table)).rows), ["BTC", "ETH"]);
    // SOL becomes the biggest: it comes in at the top, ETH drops out.
    table.set("SOL", 2, Cell::Integer(99));
    let change = delta(view.poll(&table));
    assert_eq!(keys(&change.added), ["SOL"]);
    assert_eq!(change.removed, ["ETH"]);
    assert_eq!(change.order, ["SOL", "BTC"]);
    // BTC overtakes SOL: same rows, new order.
    table.set("BTC", 2, Cell::Integer(100));
    let change = delta(view.poll(&table));
    assert!(change.added.is_empty() && change.removed.is_empty());
    assert_eq!(change.order, ["BTC", "SOL"]);
    assert_eq!(change.updated.len(), 1);
}

#[test]
fn a_new_row_out_of_range_still_counts() {
    let mut table = prices();
    let spec = ViewportSpec { limit: 1, ..spec() };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    view.poll(&table);
    table.upsert("XRP", vec![Cell::Text("XRP".into())]);
    let change = delta(view.poll(&table));
    assert_eq!(change.total_rows, 4);
    assert!(change.added.is_empty() && change.updated.is_empty() && change.order.is_empty());
}

#[test]
fn a_removed_row_leaves_the_range() {
    let mut table = prices();
    let mut view = Viewport::open(1, &spec(), &table).expect("opens");
    view.poll(&table);
    table.remove("ETH");
    let change = delta(view.poll(&table));
    assert_eq!(change.removed, ["ETH"]);
    assert_eq!(change.order, ["BTC", "SOL"]);
    assert_eq!(change.total_rows, 2);
}

#[test]
fn a_reset_sends_a_new_snapshot() {
    let table = prices();
    let mut view = Viewport::open(1, &spec(), &table).expect("opens");
    view.poll(&table);
    view.reset();
    let again = snapshot(view.poll(&table));
    assert_eq!(again.seq, 2);
    assert_eq!(again.rows.len(), 3);
}

#[test]
fn a_bad_spec_says_why() {
    let table = prices();
    let unknown_column = ViewportSpec {
        columns: vec!["ask".into()],
        ..spec()
    };
    assert_eq!(
        Viewport::open(1, &unknown_column, &table).map(|_| ()),
        Err(ViewError::UnknownColumn("ask".into()))
    );
    let unknown_sort = ViewportSpec {
        sort: vec![Sort {
            column: "size".into(),
            descending: false,
        }],
        ..spec()
    };
    assert_eq!(
        Viewport::open(1, &unknown_sort, &table).map(|_| ()),
        Err(ViewError::UnknownColumn("size".into()))
    );
    let bad_filter = ViewportSpec {
        filter: "volume >".into(),
        ..spec()
    };
    assert!(matches!(
        Viewport::open(1, &bad_filter, &table),
        Err(ViewError::Filter(_))
    ));
    assert_eq!(
        ViewError::UnknownTable("x".into()).to_string(),
        "there is no table x"
    );
}

#[test]
fn the_row_limit_is_capped() {
    let mut table = Table::new("t", &["n"]);
    for n in 0..(MAX_ROWS + 5) {
        table.upsert(format!("{n:05}"), vec![Cell::Integer(1)]);
    }
    for limit in [0, 5000] {
        let spec = ViewportSpec {
            table: "t".into(),
            limit,
            ..ViewportSpec::default()
        };
        let mut view = Viewport::open(1, &spec, &table).expect("opens");
        let first = snapshot(view.poll(&table));
        assert_eq!(first.rows.len(), MAX_ROWS);
        assert_eq!(first.total_rows as usize, MAX_ROWS + 5);
    }
}

#[test]
fn a_decimal_too_big_for_the_wire_goes_as_an_empty_cell() {
    let mut table = Table::new("t", &["n"]);
    let huge = fusion_domain::Decimal::from_i128_with_scale(i128::from(i64::MAX) + 1, 0);
    table.upsert("k", vec![Cell::Decimal(huge)]);
    let spec = ViewportSpec {
        table: "t".into(),
        ..ViewportSpec::default()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    let first = snapshot(view.poll(&table));
    assert_eq!(first.rows[0].cells[0], v1::Value::default());
}

/// What one pass over 500 rows costs, with a filter and a sort. Run it by
/// hand: `cargo test -p fusion-viewserver --release -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn cost_of_a_pass_over_500_rows() {
    let mut table = Table::new("prices", &["symbol", "bid", "volume"]);
    for n in 0..500 {
        table.upsert(
            format!("S{n:03}"),
            vec![
                Cell::Text(format!("S{n:03}")),
                dec("100.25"),
                Cell::Integer(n),
            ],
        );
    }
    for filter in ["", "volume >= 100 && bid > 50.0"] {
        let spec = ViewportSpec {
            filter: filter.into(),
            sort: by_volume(true),
            limit: 50,
            ..spec()
        };
        let mut view = Viewport::open(1, &spec, &table).expect("opens");
        view.poll(&table);
        let passes = 200;
        let start = std::time::Instant::now();
        for n in 0..passes {
            table.set("S250", 2, Cell::Integer(1000 + n));
            assert!(view.poll(&table).is_some());
        }
        let each = start.elapsed() / u32::try_from(passes).expect("small");
        println!("one pass over 500 rows, filter {filter:?}: {each:?}");
    }
}

#[test]
fn a_row_that_stops_matching_leaves_and_one_that_starts_comes_in() {
    let mut table = prices();
    let spec = ViewportSpec {
        filter: "volume >= 20".to_owned(),
        ..spec()
    };
    let mut view = Viewport::open(1, &spec, &table).expect("opens");
    assert_eq!(keys(&snapshot(view.poll(&table)).rows), ["BTC", "ETH"]);
    table.set("ETH", 2, Cell::Integer(5));
    table.set("SOL", 2, Cell::Integer(25));
    let change = delta(view.poll(&table));
    assert_eq!(change.removed, ["ETH"]);
    assert_eq!(keys(&change.added), ["SOL"]);
    // A removed row is forgotten, and a new one with its key starts fresh.
    table.remove("SOL");
    assert_eq!(delta(view.poll(&table)).removed, ["SOL"]);
    assert_eq!(
        view.passed.len(),
        table.len(),
        "the removed row is forgotten"
    );
    table.upsert(
        "SOL",
        vec![Cell::Text("SOL".into()), dec("10"), Cell::Integer(1)],
    );
    // Its old answer (it passed) is not used: it is filtered out now, so
    // nothing in view changed.
    assert!(view.poll(&table).is_none());
}
