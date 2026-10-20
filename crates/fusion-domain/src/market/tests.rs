use super::*;

const BTC: InstrumentId = InstrumentId(1);

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn snapshot(update_id: u64) -> BookSnapshot {
    BookSnapshot {
        instrument: BTC,
        update_id,
        bids: vec![
            level("100.00", "1"),
            level("99.50", "2"),
            level("99.00", "0"),
        ],
        asks: vec![level("100.50", "3"), level("101.00", "4")],
    }
}

fn delta(first: u64, last: u64, bids: Vec<Level>, asks: Vec<Level>) -> BookDelta {
    BookDelta {
        instrument: BTC,
        first_update: first,
        last_update: last,
        time: Timestamp::UNIX_EPOCH,
        bids,
        asks,
    }
}

#[test]
fn a_book_starts_from_a_snapshot_without_empty_levels() {
    let book = OrderBook::from_snapshot(&snapshot(10));
    assert_eq!(book.update_id(), 10);
    assert_eq!(book.depth(), (2, 2));
    assert_eq!(book.best_bid(), Some(level("100.00", "1")));
    assert_eq!(book.best_ask(), Some(level("100.50", "3")));
    let bids: Vec<Level> = book.bids().collect();
    assert_eq!(bids, [level("100.00", "1"), level("99.50", "2")]);
    let asks: Vec<Level> = book.asks().collect();
    assert_eq!(asks, [level("100.50", "3"), level("101.00", "4")]);
}

#[test]
fn a_delta_sets_and_removes_levels() {
    let mut book = OrderBook::from_snapshot(&snapshot(10));
    let change = delta(
        11,
        13,
        vec![level("100.00", "0"), level("99.75", "5")],
        vec![level("100.50", "1.5")],
    );
    assert_eq!(book.apply(&change), Ok(DeltaOutcome::Applied));
    assert_eq!(book.update_id(), 13);
    assert_eq!(book.best_bid(), Some(level("99.75", "5")));
    assert_eq!(book.best_ask(), Some(level("100.50", "1.5")));
}

#[test]
fn the_first_delta_may_start_before_the_snapshot() {
    // Binance: the first delta after a snapshot holds the snapshot's number.
    let mut book = OrderBook::from_snapshot(&snapshot(10));
    assert_eq!(
        book.apply(&delta(8, 12, vec![], vec![])),
        Ok(DeltaOutcome::Applied)
    );
    assert_eq!(book.update_id(), 12);
}

#[test]
fn a_delta_the_book_already_has_changes_nothing() {
    let mut book = OrderBook::from_snapshot(&snapshot(10));
    let before = book.clone();
    let old = delta(5, 10, vec![level("100.00", "9")], vec![]);
    assert_eq!(book.apply(&old), Ok(DeltaOutcome::Stale));
    assert_eq!(book, before);
}

#[test]
fn a_missing_update_is_a_gap() {
    let mut book = OrderBook::from_snapshot(&snapshot(10));
    let late = delta(12, 14, vec![level("100.00", "9")], vec![]);
    assert_eq!(
        book.apply(&late),
        Err(BookGap {
            expected: 11,
            got: 12
        })
    );
    assert_eq!(book.update_id(), 10, "a gap changes nothing");
    assert_eq!(book.best_bid(), Some(level("100.00", "1")));
}

#[test]
fn an_empty_book_has_no_best_prices() {
    let book = OrderBook::default();
    assert_eq!(book.best_bid(), None);
    assert_eq!(book.best_ask(), None);
    assert_eq!(book.depth(), (0, 0));
}

#[test]
fn the_digest_tells_books_apart_and_ignores_trailing_zeros() {
    let book = OrderBook::from_snapshot(&snapshot(10));
    let same = OrderBook::from_snapshot(&BookSnapshot {
        bids: vec![level("100.0000", "1.00"), level("99.5", "2")],
        asks: vec![level("100.5", "3"), level("101", "4.000")],
        ..snapshot(10)
    });
    assert_eq!(book.digest(), same.digest());

    let later = OrderBook::from_snapshot(&snapshot(11));
    assert_ne!(book.digest(), later.digest(), "the update number counts");

    let mut changed = book.clone();
    changed
        .apply(&delta(11, 11, vec![], vec![level("101.00", "4.1")]))
        .expect("applies");
    let mut renumbered = book.clone();
    renumbered
        .apply(&delta(11, 11, vec![], vec![]))
        .expect("applies");
    assert_ne!(changed.digest(), renumbered.digest(), "a quantity counts");

    let mut sides = book.clone();
    sides
        .apply(&delta(
            11,
            11,
            vec![level("101.00", "4")],
            vec![level("101.00", "0")],
        ))
        .expect("applies");
    let flipped = OrderBook::from_snapshot(&BookSnapshot {
        update_id: 11,
        bids: vec![
            level("101.00", "4"),
            level("100.00", "1"),
            level("99.50", "2"),
        ],
        asks: vec![level("100.50", "3")],
        ..snapshot(11)
    });
    assert_eq!(sides.digest(), flipped.digest());
    let swapped = OrderBook::from_snapshot(&BookSnapshot {
        update_id: 11,
        bids: vec![level("100.00", "1"), level("99.50", "2")],
        asks: vec![level("100.50", "3"), level("101.00", "4")],
        ..snapshot(11)
    });
    assert_ne!(sides.digest(), swapped.digest(), "the side counts");
}

#[test]
fn the_digest_is_fixed_for_all_builds() {
    // FNV-1a of known input: a change here breaks every stored recording.
    assert_eq!(OrderBook::default().digest(), 0x696c_f2cc_20a0_18ac);
}

#[test]
fn trade_numbers_show_gaps_and_repeats() {
    let mut seq = TradeSequence::default();
    assert_eq!(seq.last(), None);
    assert_eq!(seq.observe(100), Seen::First);
    assert_eq!(seq.observe(101), Seen::Next);
    assert_eq!(seq.observe(101), Seen::Repeat);
    assert_eq!(seq.observe(99), Seen::Repeat);
    assert_eq!(seq.observe(105), Seen::Gap(102..105));
    assert_eq!(seq.last(), Some(105));
    assert_eq!(seq.observe(106), Seen::Next);
}

#[test]
fn a_trade_is_plain_data() {
    let trade = Trade {
        instrument: BTC,
        id: 7,
        price: "100".parse().expect("a price"),
        quantity: "0.5".parse().expect("a quantity"),
        taker: Side::Buy,
        time: Timestamp::UNIX_EPOCH,
    };
    let top = TopOfBook {
        instrument: BTC,
        update_id: 3,
        bid: level("99", "1"),
        ask: level("101", "1"),
    };
    assert_eq!(trade, trade.clone());
    assert!(top.bid.price < top.ask.price);
}

#[test]
fn every_event_names_its_instrument() {
    let eth = InstrumentId(2);
    let trade = Trade {
        instrument: eth,
        id: 1,
        price: "1".parse().expect("a price"),
        quantity: "1".parse().expect("a quantity"),
        taker: Side::Sell,
        time: Timestamp::UNIX_EPOCH,
    };
    let top = TopOfBook {
        instrument: eth,
        update_id: 1,
        bid: level("1", "1"),
        ask: level("2", "1"),
    };
    let events = [
        MarketEvent::Trade(trade),
        MarketEvent::Top(top),
        MarketEvent::Snapshot(BookSnapshot {
            instrument: eth,
            ..snapshot(1)
        }),
        MarketEvent::Delta(BookDelta {
            instrument: eth,
            ..delta(1, 1, vec![], vec![])
        }),
    ];
    assert!(events.iter().all(|e| e.instrument() == eth));
}
