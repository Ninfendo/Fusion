use fusion_protocol::v1::{self, Delta, Row, Snapshot};

use super::*;

fn value(cell: Cell) -> v1::Value {
    v1::Value::try_from(&cell).expect("fits")
}

fn d(text: &str) -> Cell {
    Cell::Decimal(text.parse().expect("a decimal"))
}

fn snapshot(viewport: u32, columns: &[&str], rows: Vec<(&str, Vec<Cell>)>) -> ServerMessage {
    ServerMessage {
        body: Some(server_message::Body::Snapshot(Snapshot {
            viewport,
            seq: 1,
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            total_rows: u32::try_from(rows.len()).expect("small"),
            rows: rows
                .into_iter()
                .map(|(key, cells)| Row {
                    key: key.to_owned(),
                    cells: cells.into_iter().map(value).collect(),
                })
                .collect(),
        })),
    }
}

fn created(message: &ClientMessage) -> (u32, &str) {
    match &message.body {
        Some(client_message::Body::CreateViewport(c)) => {
            (c.viewport, c.spec.as_ref().expect("a spec").table.as_str())
        }
        other => panic!("not a create: {other:?}"),
    }
}

fn prices() -> ServerMessage {
    snapshot(
        1,
        &["symbol", "bid", "ask", "last", "open"],
        vec![
            (
                "BTCUSDT",
                vec![
                    Cell::Text("BTCUSDT".into()),
                    d("100"),
                    d("101"),
                    d("100.5"),
                    d("90"),
                ],
            ),
            (
                "ETHUSDT",
                vec![
                    Cell::Text("ETHUSDT".into()),
                    d("50"),
                    d("51"),
                    Cell::Empty,
                    Cell::Empty,
                ],
            ),
            (
                "SOLUSDT",
                vec![
                    Cell::Text("SOLUSDT".into()),
                    Cell::Empty,
                    Cell::Empty,
                    Cell::Empty,
                    Cell::Empty,
                ],
            ),
        ],
    )
}

fn set(symbols: &[&str]) -> BTreeSet<String> {
    symbols.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn a_connection_opens_the_prices_viewport() {
    let mut market = LiveMarket::default();
    let sent = market.on_connected();
    assert_eq!(sent.len(), 1);
    assert_eq!(created(&sent[0]), (1, "prices"));
    assert_eq!(sent[0].request, 1);
}

#[test]
fn quotes_come_from_the_prices_rows() {
    let mut market = LiveMarket::default();
    market.on_connected();
    assert!(market.on_message(prices()).is_empty());
    let quotes = market.quotes();
    assert_eq!(quotes.len(), 2, "SOL has no bid yet");
    assert_eq!(quotes[0].symbol, "BTCUSDT");
    assert_eq!(quotes[0].last, Price::new("100.5".parse().expect("p")));
    assert_eq!(quotes[0].open, Price::new("90".parse().expect("p")));
    assert_eq!(quotes[1].last, quotes[1].bid, "no trade yet: the bid");
    assert_eq!(quotes[1].open, quotes[1].bid);
    assert_eq!(market.symbols(), ["BTCUSDT", "ETHUSDT", "SOLUSDT"]);
}

#[test]
fn book_viewports_follow_what_panels_show() {
    let mut market = LiveMarket::default();
    let open = market.want_books(&set(&["BTCUSDT"]));
    assert_eq!(created(&open[0]), (2, "book/BTCUSDT"));
    assert_eq!(open[0].request, 1, "requests are numbered from 1");
    assert!(
        market.want_books(&set(&["BTCUSDT"])).is_empty(),
        "already open"
    );
    let swap = market.want_books(&set(&["ETHUSDT"]));
    assert!(matches!(
        swap[0].body,
        Some(client_message::Body::RemoveViewport(RemoveViewport {
            viewport: 2
        }))
    ));
    assert_eq!(created(&swap[1]), (3, "book/ETHUSDT"));
    assert_eq!(
        (swap[0].request, swap[1].request),
        (2, 3),
        "each request has its own number"
    );
    // A new connection opens them all again, with the same numbers.
    let again = market.on_connected();
    assert_eq!(created(&again[1]), (3, "book/ETHUSDT"));
}

#[test]
fn a_book_comes_from_its_level_rows() {
    let mut market = LiveMarket::default();
    market.want_books(&set(&["BTCUSDT"]));
    let columns = ["side", "level", "price", "quantity"];
    let level = |side: &str, n: i64, p: Cell, q: Cell| {
        vec![Cell::Text(side.into()), Cell::Integer(n), p, q]
    };
    market.on_message(snapshot(
        2,
        &columns,
        vec![
            ("b00", level("bid", 0, d("100"), d("1"))),
            ("b01", level("bid", 1, d("99"), d("2"))),
            ("b02", level("bid", 2, Cell::Empty, Cell::Empty)),
            ("a00", level("ask", 0, d("101"), d("3"))),
        ],
    ));
    let book = market.book("BTCUSDT", 5).expect("a book");
    assert_eq!(book.bids.len(), 2, "stops at the first empty level");
    assert_eq!(book.asks.len(), 1);
    assert_eq!(
        book.bids[1].quantity,
        Quantity::new("2".parse().expect("q"))
    );
    assert_eq!(market.book("BTCUSDT", 1).map(|b| b.bids.len()), Some(1));
    assert_eq!(market.book("ETHUSDT", 5), None);
}

#[test]
fn a_missed_delta_asks_for_a_new_snapshot() {
    let mut market = LiveMarket::default();
    market.on_connected();
    market.want_books(&set(&["BTCUSDT"]));
    market.on_message(prices());
    let late = |viewport| ServerMessage {
        body: Some(server_message::Body::Delta(Delta {
            viewport,
            seq: 5,
            ..Delta::default()
        })),
    };
    let ask = market.on_message(late(1));
    assert!(matches!(
        &ask[0].body,
        Some(client_message::Body::ChangeViewport(c))
            if c.viewport == 1 && c.spec.as_ref().is_some_and(|s| s.table == "prices")
    ));
    let ask = market.on_message(late(2));
    assert!(matches!(
        &ask[0].body,
        Some(client_message::Body::ChangeViewport(c))
            if c.viewport == 2 && c.spec.as_ref().is_some_and(|s| s.table == "book/BTCUSDT")
    ));
}

#[test]
fn messages_for_nobody_and_answers_change_nothing() {
    let mut market = LiveMarket::default();
    market.on_connected();
    assert!(market.on_message(snapshot(9, &["x"], vec![])).is_empty());
    let late = ServerMessage {
        body: Some(server_message::Body::Delta(Delta {
            viewport: 9,
            seq: 7,
            ..Delta::default()
        })),
    };
    assert!(market.on_message(late).is_empty());
    let ack = ServerMessage {
        body: Some(server_message::Body::Ack(v1::Ack {
            request: 1,
            result: None,
        })),
    };
    assert!(market.on_message(ack).is_empty());
    assert!(market.quotes().is_empty());
}
