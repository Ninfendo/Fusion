use super::*;

/// Messages captured from the Spot Testnet on 2026-10-20.
const STREAM: &str = include_str!("../../tests/fixtures/testnet-stream.jsonl");
const SNAPSHOT: &str = include_str!("../../tests/fixtures/testnet-depth-snapshot.json");
const TRADES: &str = include_str!("../../tests/fixtures/testnet-agg-trades.json");

const BTC: InstrumentId = InstrumentId(1);

fn symbols() -> SymbolTable {
    SymbolTable::new(["BTCUSDT"])
}

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn events() -> Vec<MarketEvent> {
    STREAM
        .lines()
        .map(|line| match parse_stream(line, &symbols()) {
            Ok(Message::Event(event)) => event,
            other => panic!("not an event: {other:?}"),
        })
        .collect()
}

#[test]
fn every_captured_message_reads() {
    assert_eq!(events().len(), 5);
}

#[test]
fn a_depth_update_becomes_a_delta() {
    let MarketEvent::Delta(delta) = &events()[0] else {
        panic!("a delta");
    };
    assert_eq!(delta.instrument, BTC);
    assert_eq!(
        (delta.first_update, delta.last_update),
        (6_714_536, 6_714_537)
    );
    assert_eq!(
        delta.time,
        Timestamp::from_unix_millis(1_792_517_604_317).expect("a time")
    );
    assert!(delta.bids.is_empty());
    assert_eq!(
        delta.asks,
        [level("82831.87", "0"), level("82834.80", "0.0006")]
    );
}

#[test]
fn a_book_ticker_becomes_the_best_prices() {
    let MarketEvent::Top(top) = &events()[3] else {
        panic!("a top");
    };
    assert_eq!(top.update_id, 6_714_542);
    assert_eq!(top.bid, level("82738.38", "4.6861"));
    assert_eq!(top.ask, level("82738.39", "28.30676"));
}

#[test]
fn an_agg_trade_becomes_a_trade() {
    let MarketEvent::Trade(trade) = &events()[4] else {
        panic!("a trade");
    };
    assert_eq!(trade.id, 971_133);
    assert_eq!(trade.price, "82738.91".parse().expect("a price"));
    assert_eq!(trade.quantity, "0.001".parse().expect("a quantity"));
    assert_eq!(trade.taker, Side::Buy, "m=false: the buyer took liquidity");
    assert_eq!(
        trade.time,
        Timestamp::from_unix_millis(1_792_517_609_606).expect("a time")
    );
}

#[test]
fn rest_snapshots_and_trades_read() {
    let snapshot = parse_snapshot(SNAPSHOT, BTC).expect("reads");
    assert_eq!(snapshot.update_id, 6_714_599);
    assert_eq!(snapshot.bids.len(), 5);
    assert_eq!(snapshot.bids[0], level("82757.18", "28.01144"));
    assert_eq!(snapshot.asks[0], level("82757.19", "7.31343"));

    let trades = parse_trades(TRADES, BTC).expect("reads");
    let ids: Vec<u64> = trades.iter().map(|t| t.id).collect();
    assert_eq!(ids, [971_132, 971_133, 971_134]);
    assert_eq!(
        trades[2].taker,
        Side::Sell,
        "m=true: the seller took liquidity"
    );
}

#[test]
fn the_raw_depth_stream_name_reads_too() {
    let line = STREAM
        .lines()
        .next()
        .expect("a line")
        .replace("btcusdt@depth@100ms", "btcusdt@depth");
    assert!(matches!(
        parse_stream(&line, &symbols()),
        Ok(Message::Event(MarketEvent::Delta(_)))
    ));
}

#[test]
fn a_server_shutdown_says_so() {
    let line = r#"{"stream":"btcusdt@aggTrade","data":{"e":"serverShutdown","E":1770123456789}}"#;
    assert_eq!(parse_stream(line, &symbols()), Ok(Message::Shutdown));
    let raw = r#"{"e":"serverShutdown","E":1770123456789}"#;
    assert_eq!(parse_stream(raw, &symbols()), Ok(Message::Shutdown));
}

#[test]
fn bad_messages_say_why() {
    let table = symbols();
    assert!(matches!(
        parse_stream("{ nope", &table),
        Err(ParseError::Json(_))
    ));
    let other_symbol = STREAM
        .lines()
        .next()
        .expect("a line")
        .replace("btcusdt@", "solusdt@");
    assert_eq!(
        parse_stream(&other_symbol, &table),
        Err(ParseError::UnknownStream("solusdt@depth@100ms".to_owned()))
    );
    let no_at = r#"{"stream":"btcusdt","data":{}}"#;
    assert_eq!(
        parse_stream(no_at, &table),
        Err(ParseError::UnknownStream("btcusdt".to_owned()))
    );
    let other_kind = r#"{"stream":"btcusdt@kline_1m","data":{}}"#;
    assert_eq!(
        parse_stream(other_kind, &table),
        Err(ParseError::UnknownStream("btcusdt@kline_1m".to_owned()))
    );
    let bad_price = STREAM
        .lines()
        .nth(4)
        .expect("a trade")
        .replace("82738.91000000", "lots");
    assert_eq!(
        parse_stream(&bad_price, &table),
        Err(ParseError::Number("lots".to_owned()))
    );
    let bad_time = r#"{"stream":"btcusdt@aggTrade","data":{"a":1,"p":"1","q":"1","T":9223372036854775807,"m":true}}"#;
    assert!(matches!(
        parse_stream(bad_time, &table),
        Err(ParseError::Number(_))
    ));
    assert!(
        ParseError::Json("x".to_owned())
            .to_string()
            .contains("not a Binance message")
    );
}
