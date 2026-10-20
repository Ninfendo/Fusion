use fusion_domain::{Level, Side, Timestamp};

use super::*;

const BTC: InstrumentId = InstrumentId(1);
const ETH: InstrumentId = InstrumentId(2);

fn feed() -> Feed {
    Feed::new(&SymbolTable::new(["BTCUSDT", "ETHUSDT"]))
}

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn delta(first: u64, last: u64, bid: &str) -> BookDelta {
    BookDelta {
        instrument: BTC,
        first_update: first,
        last_update: last,
        time: Timestamp::UNIX_EPOCH,
        bids: vec![level(bid, "1")],
        asks: vec![],
    }
}

fn snapshot(update_id: u64) -> BookSnapshot {
    BookSnapshot {
        instrument: BTC,
        update_id,
        bids: vec![level("100", "1")],
        asks: vec![level("101", "1")],
    }
}

fn trade(id: u64) -> Trade {
    Trade {
        instrument: BTC,
        id,
        price: "100".parse().expect("a price"),
        quantity: "1".parse().expect("a quantity"),
        taker: Side::Buy,
        time: Timestamp::UNIX_EPOCH,
    }
}

fn top(update_id: u64) -> TopOfBook {
    TopOfBook {
        instrument: BTC,
        update_id,
        bid: level("100", "1"),
        ask: level("101", "1"),
    }
}

/// Feeds stream events and returns what came out.
fn run(feed: &mut Feed, events: impl IntoIterator<Item = MarketEvent>) -> Output {
    let mut out = Output::default();
    for event in events {
        feed.on_event(event, &mut out);
    }
    out
}

fn delta_ranges(out: &Output) -> Vec<(u64, u64)> {
    out.events
        .iter()
        .filter_map(|e| match e {
            MarketEvent::Delta(d) => Some((d.first_update, d.last_update)),
            _ => None,
        })
        .collect()
}

/// A feed whose BTC book is live at update 10.
fn live_feed() -> Feed {
    let mut feed = feed();
    run(&mut feed, [MarketEvent::Delta(delta(9, 10, "99"))]);
    feed.on_snapshot(snapshot(9), &mut Output::default());
    assert!(feed.book(BTC).is_some(), "live");
    feed
}

#[test]
fn the_first_change_asks_for_a_snapshot_once() {
    let mut feed = feed();
    let out = run(
        &mut feed,
        [
            MarketEvent::Delta(delta(5, 6, "99")),
            MarketEvent::Delta(delta(7, 8, "98")),
        ],
    );
    assert_eq!(out.wants, [Want::Snapshot(BTC)]);
    assert!(out.events.is_empty(), "nothing passes before the snapshot");
    assert!(feed.book(BTC).is_none());
}

#[test]
fn a_snapshot_starts_the_book_and_replays_the_buffer() {
    let mut feed = feed();
    run(
        &mut feed,
        [
            MarketEvent::Delta(delta(5, 6, "99")),
            MarketEvent::Delta(delta(7, 8, "98")),
            MarketEvent::Delta(delta(9, 10, "97")),
        ],
    );
    let mut out = Output::default();
    feed.on_snapshot(snapshot(7), &mut out);
    assert!(matches!(out.events[0], MarketEvent::Snapshot(_)));
    assert_eq!(
        delta_ranges(&out),
        [(7, 8), (9, 10)],
        "5-6 is older than the snapshot"
    );
    let book = feed.book(BTC).expect("live");
    assert_eq!(book.update_id(), 10);
    assert_eq!(book.best_bid(), Some(level("100", "1")));

    let more = run(&mut feed, [MarketEvent::Delta(delta(11, 12, "102"))]);
    assert_eq!(delta_ranges(&more), [(11, 12)]);
    assert_eq!(
        feed.book(BTC).and_then(OrderBook::best_bid),
        Some(level("102", "1"))
    );
}

#[test]
fn a_snapshot_older_than_the_buffer_is_asked_for_again() {
    let mut feed = feed();
    run(&mut feed, [MarketEvent::Delta(delta(20, 21, "99"))]);
    let mut out = Output::default();
    feed.on_snapshot(snapshot(10), &mut out);
    assert_eq!(out.notes, [Note::SnapshotTooOld(BTC)]);
    assert_eq!(out.wants, [Want::Snapshot(BTC)]);
    assert!(out.events.is_empty());
    assert!(feed.book(BTC).is_none());
    // The buffer is kept for the next snapshot.
    let mut next = Output::default();
    feed.on_snapshot(snapshot(20), &mut next);
    assert_eq!(delta_ranges(&next), [(20, 21)]);
}

#[test]
fn a_snapshot_without_buffered_changes_starts_the_book() {
    let mut feed = feed();
    let mut out = Output::default();
    feed.on_snapshot(snapshot(10), &mut out);
    assert_eq!(feed.book(BTC).map(OrderBook::update_id), Some(10));
}

#[test]
fn a_gap_restarts_the_book_from_a_new_snapshot() {
    let mut feed = live_feed();
    let out = run(&mut feed, [MarketEvent::Delta(delta(15, 16, "98"))]);
    assert_eq!(
        out.notes,
        [Note::BookGap(
            BTC,
            BookGap {
                expected: 11,
                got: 15
            }
        )]
    );
    assert_eq!(out.wants, [Want::Snapshot(BTC)]);
    assert!(feed.book(BTC).is_none());
    // The delta after the gap waits for the snapshot; no second request.
    let waiting = run(&mut feed, [MarketEvent::Delta(delta(17, 18, "97"))]);
    assert!(waiting.wants.is_empty());
    let mut out = Output::default();
    feed.on_snapshot(snapshot(15), &mut out);
    assert_eq!(delta_ranges(&out), [(15, 16), (17, 18)]);
}

#[test]
fn a_gap_inside_the_buffer_waits_for_another_snapshot() {
    let mut feed = feed();
    run(
        &mut feed,
        [
            MarketEvent::Delta(delta(5, 6, "99")),
            MarketEvent::Delta(delta(9, 10, "98")),
        ],
    );
    let mut out = Output::default();
    feed.on_snapshot(snapshot(6), &mut out);
    assert!(matches!(out.events[0], MarketEvent::Snapshot(_)));
    assert!(matches!(out.notes[0], Note::BookGap(BTC, _)));
    assert_eq!(out.wants, [Want::Snapshot(BTC)]);
    assert!(feed.book(BTC).is_none());
    let mut again = Output::default();
    feed.on_snapshot(snapshot(9), &mut again);
    assert_eq!(delta_ranges(&again), [(9, 10)], "the kept change applies");
}

#[test]
fn repeated_changes_and_snapshots_are_dropped() {
    let mut feed = live_feed();
    let out = run(&mut feed, [MarketEvent::Delta(delta(9, 10, "50"))]);
    assert!(out.events.is_empty() && out.notes.is_empty());
    let mut late = Output::default();
    feed.on_snapshot(snapshot(10), &mut late);
    assert!(late.events.is_empty(), "a live book ignores snapshots");
}

#[test]
fn a_full_buffer_starts_over() {
    let mut feed = feed();
    let full = MAX_BUFFERED as u64 + 1;
    // Changes 1 to full: the last one finds the buffer full and starts it again.
    let changes = (1..=full).map(|n| MarketEvent::Delta(delta(n, n, "99")));
    let out = run(&mut feed, changes);
    assert_eq!(out.notes, [Note::BufferFull(BTC)]);
    run(
        &mut feed,
        [MarketEvent::Delta(delta(full + 1, full + 1, "98"))],
    );
    let mut snap = Output::default();
    feed.on_snapshot(snapshot(full), &mut snap);
    assert_eq!(
        delta_ranges(&snap),
        [(full + 1, full + 1)],
        "only the changes after the restart are kept"
    );
}

#[test]
fn trades_pass_once_in_order() {
    let mut feed = feed();
    let out = run(
        &mut feed,
        [100, 101, 101, 102].map(|id| MarketEvent::Trade(trade(id))),
    );
    let ids: Vec<u64> = out
        .events
        .iter()
        .filter_map(|e| match e {
            MarketEvent::Trade(t) => Some(t.id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, [100, 101, 102]);
    assert!(out.wants.is_empty());
}

#[test]
fn missed_trades_are_fetched_and_passed_once() {
    let mut feed = feed();
    let out = run(
        &mut feed,
        [100, 105].map(|id| MarketEvent::Trade(trade(id))),
    );
    assert_eq!(out.notes, [Note::MissedTrades(BTC, 101..105)]);
    assert_eq!(
        out.wants,
        [Want::Trades {
            instrument: BTC,
            from: 101
        }]
    );

    // The first page brings 101 and 102 (and 105, already passed on).
    let mut first = Output::default();
    feed.on_trades(BTC, vec![trade(101), trade(102), trade(105)], &mut first);
    assert_eq!(first.events.len(), 2);
    assert_eq!(
        first.wants,
        [Want::Trades {
            instrument: BTC,
            from: 103
        }]
    );

    let mut rest = Output::default();
    feed.on_trades(BTC, vec![trade(103), trade(104)], &mut rest);
    assert_eq!(rest.events.len(), 2);
    assert!(rest.wants.is_empty(), "nothing is missing any more");
}

#[test]
fn a_trade_in_the_middle_of_a_gap_splits_it() {
    let mut feed = feed();
    run(
        &mut feed,
        [100, 110].map(|id| MarketEvent::Trade(trade(id))),
    );
    let mut out = Output::default();
    feed.on_trades(BTC, vec![trade(105)], &mut out);
    assert_eq!(out.events.len(), 1);
    assert_eq!(
        out.wants,
        [Want::Trades {
            instrument: BTC,
            from: 101
        }]
    );
    let mut low = Output::default();
    feed.on_trades(BTC, (101..105).map(trade).collect(), &mut low);
    assert_eq!(
        low.wants,
        [Want::Trades {
            instrument: BTC,
            from: 106
        }]
    );
}

#[test]
fn trades_the_api_no_longer_has_are_given_up() {
    let mut feed = feed();
    run(
        &mut feed,
        [100, 103].map(|id| MarketEvent::Trade(trade(id))),
    );
    let mut out = Output::default();
    feed.on_trades(BTC, vec![], &mut out);
    let lost: Vec<std::ops::Range<u64>> = std::iter::once(101..103).collect();
    assert_eq!(out.notes, [Note::TradesLost(BTC, lost)]);
    assert!(out.wants.is_empty());
}

#[test]
fn best_prices_pass_only_when_newer() {
    let mut feed = feed();
    let out = run(&mut feed, [5, 5, 4, 6].map(|u| MarketEvent::Top(top(u))));
    let ids: Vec<u64> = out
        .events
        .iter()
        .filter_map(|e| match e {
            MarketEvent::Top(t) => Some(t.update_id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, [5, 6]);
}

#[test]
fn instruments_are_kept_apart_and_unknown_ones_ignored() {
    let mut feed = live_feed();
    let eth_delta = BookDelta {
        instrument: ETH,
        ..delta(1, 2, "50")
    };
    let out = run(&mut feed, [MarketEvent::Delta(eth_delta)]);
    assert_eq!(out.wants, [Want::Snapshot(ETH)]);
    assert!(feed.book(BTC).is_some() && feed.book(ETH).is_none());

    let stranger = Trade {
        instrument: InstrumentId(9),
        ..trade(1)
    };
    assert_eq!(
        run(&mut feed, [MarketEvent::Trade(stranger)]),
        Output::default()
    );
    let mut out = Output::default();
    feed.on_snapshot(
        BookSnapshot {
            instrument: InstrumentId(0),
            ..snapshot(1)
        },
        &mut out,
    );
    feed.on_trades(InstrumentId(9), vec![trade(1)], &mut out);
    assert_eq!(out, Output::default());
    assert!(feed.book(InstrumentId(9)).is_none());
    assert!(feed.book(InstrumentId(0)).is_none());
}

#[test]
fn a_snapshot_on_the_stream_counts_as_one_from_the_api() {
    let mut feed = feed();
    let out = run(&mut feed, [MarketEvent::Snapshot(snapshot(3))]);
    assert!(matches!(out.events[0], MarketEvent::Snapshot(_)));
    assert!(feed.book(BTC).is_some());
}
