use fusion_domain::{BookDelta, BookSnapshot, Timestamp};

use super::*;

const BTC: InstrumentId = InstrumentId(1);

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn d(text: &str) -> Cell {
    Cell::Decimal(text.parse().expect("a decimal"))
}

fn setup() -> (MarketTables, Tables) {
    let mut tables = Tables::default();
    let market = MarketTables::new(&["BTCUSDT".to_owned(), "ETHUSDT".to_owned()], &mut tables);
    (market, tables)
}

fn price_cell(tables: &Tables, column: &str) -> Cell {
    let prices = tables.get(PRICES).expect("prices");
    let at = prices.column(column).expect("a column");
    prices.row("BTCUSDT").expect("a row")[at].clone()
}

fn book_cells(tables: &Tables, key: &str) -> (Cell, Cell) {
    let book = tables.get("book/BTCUSDT").expect("a book table");
    let row = book.row(key).expect("a row");
    (row[2].clone(), row[3].clone())
}

fn trade(id: u64, price: &str, quantity: &str) -> MarketEvent {
    MarketEvent::Trade(Trade {
        instrument: BTC,
        id,
        price: price.parse().expect("a price"),
        quantity: quantity.parse().expect("a quantity"),
        taker: Side::Buy,
        time: Timestamp::from_unix_nanos(i64::try_from(id).expect("small")),
    })
}

#[test]
fn the_tables_start_with_a_row_per_instrument_and_empty_books() {
    let (_, tables) = setup();
    assert_eq!(tables.len(), 3, "prices and two books");
    let prices = tables.get(PRICES).expect("prices");
    assert_eq!(prices.columns(), PRICE_COLUMNS);
    assert_eq!(prices.len(), 2);
    assert_eq!(price_cell(&tables, "bid"), Cell::Empty);
    let book = tables.get(&book_table("BTCUSDT")).expect("a book table");
    assert_eq!(book.columns(), BOOK_COLUMNS);
    assert_eq!(book.len(), 2 * BOOK_DEPTH);
    assert_eq!(
        book.row("a03"),
        Some(
            &[
                Cell::Text("ask".into()),
                Cell::Integer(3),
                Cell::Empty,
                Cell::Empty
            ][..]
        )
    );
    assert_eq!(book_key(Side::Buy, 0), "b00");
}

#[test]
fn best_prices_fill_the_price_row() {
    let (mut market, mut tables) = setup();
    market.apply(
        &MarketEvent::Top(TopOfBook {
            instrument: BTC,
            update_id: 1,
            bid: level("100", "2"),
            ask: level("101", "3"),
        }),
        &mut tables,
    );
    assert_eq!(price_cell(&tables, "bid"), d("100"));
    assert_eq!(price_cell(&tables, "bid_qty"), d("2"));
    assert_eq!(price_cell(&tables, "ask"), d("101"));
    assert_eq!(price_cell(&tables, "ask_qty"), d("3"));
}

#[test]
fn trades_add_up() {
    let (mut market, mut tables) = setup();
    market.apply(&trade(1, "100", "1.5"), &mut tables);
    market.apply(&trade(2, "110", "0.5"), &mut tables);
    assert_eq!(price_cell(&tables, "last"), d("110"));
    assert_eq!(price_cell(&tables, "last_qty"), d("0.5"));
    assert_eq!(
        price_cell(&tables, "change"),
        Cell::Real(10.0),
        "+10% since 100"
    );
    assert_eq!(price_cell(&tables, "volume"), d("2"));
    assert_eq!(price_cell(&tables, "trades"), Cell::Integer(2));
    assert_eq!(
        price_cell(&tables, "time"),
        Cell::Time(Timestamp::from_unix_nanos(2))
    );
}

#[test]
fn a_first_price_of_zero_shows_no_change() {
    let (mut market, mut tables) = setup();
    market.apply(&trade(1, "0", "1"), &mut tables);
    market.apply(&trade(2, "5", "1"), &mut tables);
    assert_eq!(price_cell(&tables, "change"), Cell::Real(0.0));
}

#[test]
fn a_book_fills_its_table_and_follows_changes() {
    let (mut market, mut tables) = setup();
    market.apply(
        &MarketEvent::Snapshot(BookSnapshot {
            instrument: BTC,
            update_id: 10,
            bids: vec![level("100", "1"), level("99", "2")],
            asks: vec![level("101", "3")],
        }),
        &mut tables,
    );
    assert_eq!(book_cells(&tables, "b00"), (d("100"), d("1")));
    assert_eq!(book_cells(&tables, "b01"), (d("99"), d("2")));
    assert_eq!(book_cells(&tables, "a00"), (d("101"), d("3")));
    assert_eq!(book_cells(&tables, "a01"), (Cell::Empty, Cell::Empty));

    let change = |first, last, bids| {
        MarketEvent::Delta(BookDelta {
            instrument: BTC,
            first_update: first,
            last_update: last,
            time: Timestamp::UNIX_EPOCH,
            bids,
            asks: vec![],
        })
    };
    market.apply(&change(11, 11, vec![level("100", "0")]), &mut tables);
    assert_eq!(
        book_cells(&tables, "b00"),
        (d("99"), d("2")),
        "the best bid went"
    );
    assert_eq!(book_cells(&tables, "b01"), (Cell::Empty, Cell::Empty));

    // A gap drops the book until the next snapshot.
    market.apply(&change(20, 20, vec![level("98", "1")]), &mut tables);
    market.apply(&change(21, 21, vec![level("97", "1")]), &mut tables);
    assert_eq!(book_cells(&tables, "b00"), (d("99"), d("2")), "unchanged");
}

#[test]
fn a_change_before_any_snapshot_and_unknown_instruments_are_ignored() {
    let (mut market, mut tables) = setup();
    let version = tables.get("book/BTCUSDT").expect("a book").version();
    market.apply(
        &MarketEvent::Delta(BookDelta {
            instrument: BTC,
            first_update: 1,
            last_update: 1,
            time: Timestamp::UNIX_EPOCH,
            bids: vec![level("1", "1")],
            asks: vec![],
        }),
        &mut tables,
    );
    assert_eq!(
        tables.get("book/BTCUSDT").expect("a book").version(),
        version
    );
    let stranger = MarketEvent::Trade(Trade {
        instrument: InstrumentId(9),
        ..match trade(1, "1", "1") {
            MarketEvent::Trade(t) => t,
            _ => unreachable!("a trade"),
        }
    });
    market.apply(&stranger, &mut tables);
    market.apply(&trade(1, "1", "1"), &mut Tables::default());
}
