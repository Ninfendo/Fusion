use fusion_domain::{BookDelta, BookSnapshot, Level, Side, Timestamp, TopOfBook, Trade};

use super::*;
use crate::feed::Output;

const BTC: InstrumentId = InstrumentId(1);
const ETH: InstrumentId = InstrumentId(2);
const OLD: u64 = 1;
const NEW: u64 = 2;

fn symbols() -> SymbolTable {
    SymbolTable::new(["BTCUSDT", "ETHUSDT"])
}

fn overlap() -> Overlap {
    Overlap::new(OLD, NEW, Instant::now())
}

fn delta(instrument: InstrumentId, first: u64, last: u64) -> MarketEvent {
    MarketEvent::Delta(BookDelta {
        instrument,
        first_update: first,
        last_update: last,
        time: Timestamp::UNIX_EPOCH,
        bids: vec![],
        asks: vec![],
    })
}

fn trade(instrument: InstrumentId, id: u64) -> MarketEvent {
    MarketEvent::Trade(Trade {
        instrument,
        id,
        price: "1".parse().expect("a price"),
        quantity: "1".parse().expect("a quantity"),
        taker: Side::Buy,
        time: Timestamp::UNIX_EPOCH,
    })
}

fn top(instrument: InstrumentId) -> MarketEvent {
    let level = Level::new("1".parse().expect("p"), "1".parse().expect("q"));
    MarketEvent::Top(TopOfBook {
        instrument,
        update_id: 1,
        bid: level,
        ask: level,
    })
}

/// A feed whose BTC book is at update 10 and whose last BTC trade is 100.
fn feed() -> Feed {
    let mut feed = Feed::new(&symbols());
    let mut out = Output::default();
    feed.on_snapshot(
        BookSnapshot {
            instrument: BTC,
            update_id: 10,
            bids: vec![],
            asks: vec![],
        },
        &mut out,
    );
    feed.on_event(trade(BTC, 100), &mut out);
    feed
}

/// Admits `event` from `conn` and, like the live loop, applies it and
/// every parked event that fits after it. Returns what the feed took.
fn offer(o: &mut Overlap, feed: &mut Feed, conn: u64, event: MarketEvent) -> Vec<MarketEvent> {
    let mut out = Output::default();
    match o.admit(conn, &event, feed) {
        Admit::Pass => {
            feed.on_event(event, &mut out);
            while let Some(parked) = o.unpark(feed) {
                feed.on_event(parked, &mut out);
            }
        }
        Admit::Park => {
            if let Some(oldest) = o.park(event) {
                feed.on_event(oldest, &mut out);
            }
        }
        Admit::Drop => {}
    }
    out.events
}

fn ranges(events: &[MarketEvent]) -> Vec<(u64, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            MarketEvent::Delta(d) => Some((d.first_update, d.last_update)),
            MarketEvent::Trade(t) => Some((t.id, t.id)),
            _ => None,
        })
        .collect()
}

#[test]
fn numbers_fit_follow_or_come_too_early() {
    assert_eq!(fit(5, 10, 10), Admit::Drop, "the feed has up to 10");
    assert_eq!(fit(9, 11, 10), Admit::Pass, "overlaps the end");
    assert_eq!(fit(11, 12, 10), Admit::Pass, "the next one");
    assert_eq!(fit(12, 13, 10), Admit::Park, "11 is missing");
}

#[test]
fn a_new_connection_ahead_waits_for_the_old_one() {
    // The case that broke the first hand-over: the new connection
    // delivers 12-13 before the old one has delivered 11.
    let (mut o, mut feed) = (overlap(), feed());
    assert!(
        offer(&mut o, &mut feed, NEW, delta(BTC, 12, 13)).is_empty(),
        "parked"
    );
    let taken = offer(&mut o, &mut feed, OLD, delta(BTC, 11, 11));
    assert_eq!(
        ranges(&taken),
        [(11, 11), (12, 13)],
        "11, then the parked 12-13"
    );
    assert!(
        offer(&mut o, &mut feed, OLD, delta(BTC, 12, 13)).is_empty(),
        "already there"
    );
    let next = offer(&mut o, &mut feed, NEW, delta(BTC, 14, 14));
    assert_eq!(ranges(&next), [(14, 14)]);
}

#[test]
fn an_old_connection_ahead_is_fine_too() {
    let (mut o, mut feed) = (overlap(), feed());
    assert_eq!(
        ranges(&offer(&mut o, &mut feed, OLD, delta(BTC, 11, 12))),
        [(11, 12)]
    );
    assert!(offer(&mut o, &mut feed, NEW, delta(BTC, 11, 12)).is_empty());
    assert_eq!(
        ranges(&offer(&mut o, &mut feed, NEW, delta(BTC, 13, 13))),
        [(13, 13)]
    );
}

#[test]
fn trades_wait_for_the_missing_one_too() {
    let (mut o, mut feed) = (overlap(), feed());
    assert!(
        offer(&mut o, &mut feed, NEW, trade(BTC, 102)).is_empty(),
        "101 is missing"
    );
    assert!(offer(&mut o, &mut feed, NEW, trade(BTC, 103)).is_empty());
    let taken = offer(&mut o, &mut feed, OLD, trade(BTC, 101));
    assert_eq!(ranges(&taken), [(101, 101), (102, 102), (103, 103)]);
    assert!(offer(&mut o, &mut feed, OLD, trade(BTC, 102)).is_empty());
}

#[test]
fn without_a_book_or_a_trade_yet_the_old_connection_feeds() {
    let (mut o, feed) = (overlap(), feed());
    assert_eq!(o.admit(OLD, &delta(ETH, 1, 2), &feed), Admit::Pass);
    assert_eq!(o.admit(NEW, &delta(ETH, 1, 2), &feed), Admit::Drop);
    assert_eq!(o.admit(OLD, &trade(ETH, 1), &feed), Admit::Pass);
    assert_eq!(o.admit(NEW, &trade(ETH, 1), &feed), Admit::Drop);
}

#[test]
fn best_prices_pass_from_either_and_strangers_are_dropped() {
    let (mut o, feed) = (overlap(), feed());
    assert_eq!(o.admit(OLD, &top(BTC), &feed), Admit::Pass);
    assert_eq!(o.admit(NEW, &top(BTC), &feed), Admit::Pass);
    assert_eq!(o.admit(3, &top(BTC), &feed), Admit::Drop);
    assert_eq!(o.admit(3, &delta(BTC, 11, 11), &feed), Admit::Drop);
}

#[test]
fn the_overlap_is_complete_once_the_new_connection_covers_everything() {
    let (mut o, mut feed) = (overlap(), feed());
    let table = symbols();
    assert!(
        !o.complete(&table, &feed),
        "the new connection sent nothing yet"
    );
    offer(&mut o, &mut feed, NEW, delta(BTC, 11, 11));
    offer(&mut o, &mut feed, NEW, trade(BTC, 101));
    let btc_only = SymbolTable::new(["BTCUSDT"]);
    assert!(
        o.complete(&btc_only, &feed),
        "the feed used update 11 and trade 101, the first the new connection brought"
    );
    assert!(!o.complete(&table, &feed), "ETH not covered");
    // ETH: a book from a snapshot at 50, a first trade at 7.
    let mut out = Output::default();
    feed.on_snapshot(
        BookSnapshot {
            instrument: ETH,
            update_id: 50,
            bids: vec![],
            asks: vec![],
        },
        &mut out,
    );
    feed.on_event(trade(ETH, 7), &mut out);
    // The new connection starts ETH at update 52: the feed (at 50) has not
    // reached it yet.
    offer(&mut o, &mut feed, NEW, delta(ETH, 52, 52));
    offer(&mut o, &mut feed, NEW, trade(ETH, 8));
    assert!(
        !o.complete(&table, &feed),
        "ETH book update 51 still to come"
    );
    offer(&mut o, &mut feed, OLD, delta(ETH, 51, 51));
    assert!(o.complete(&table, &feed));
}

#[test]
fn trades_ahead_keep_the_overlap_open() {
    let (mut o, mut feed) = (overlap(), feed());
    let table = SymbolTable::new(["BTCUSDT"]);
    offer(&mut o, &mut feed, NEW, delta(BTC, 11, 11));
    offer(&mut o, &mut feed, NEW, trade(BTC, 102));
    assert!(!o.complete(&table, &feed), "trade 101 still to come");
    offer(&mut o, &mut feed, OLD, trade(BTC, 101));
    assert!(o.complete(&table, &feed));
}

#[test]
fn what_is_still_parked_comes_out_in_order() {
    let (mut o, mut feed) = (overlap(), feed());
    offer(&mut o, &mut feed, NEW, trade(BTC, 105));
    offer(&mut o, &mut feed, NEW, delta(BTC, 20, 21));
    offer(&mut o, &mut feed, NEW, delta(BTC, 15, 16));
    offer(&mut o, &mut feed, NEW, trade(BTC, 103));
    let drained = o.drain();
    assert_eq!(
        ranges(&drained),
        [(15, 16), (20, 21), (103, 103), (105, 105)]
    );
    assert!(o.drain().is_empty());
    assert!(o.unpark(&feed).is_none());
    let mut more = overlap();
    more.park(top(BTC));
    let snapshot = MarketEvent::Snapshot(BookSnapshot {
        instrument: BTC,
        update_id: 1,
        bids: vec![],
        asks: vec![],
    });
    more.park(snapshot);
    more.park(trade(ETH, 1));
    let kinds: Vec<u8> = more.drain().iter().map(|e| order(e).1).collect();
    assert_eq!(kinds, [2, 3, 1], "BTC first, then ETH");
}

#[test]
fn a_full_parking_gives_up_its_oldest() {
    let mut o = overlap();
    for n in 0..MAX_PARKED as u64 {
        assert!(o.park(trade(BTC, 1000 + n)).is_none());
    }
    let oldest = o.park(trade(BTC, 99_999));
    assert_eq!(oldest.map(|e| order(&e).2), Some(1000));
}
