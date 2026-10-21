use fusion_domain::{DeltaOutcome, OrderBook};

use super::*;

const T: Timestamp = Timestamp::UNIX_EPOCH;

#[test]
fn instruments_are_numbered_and_named_in_order() {
    let market = SyntheticMarket::new(3, 1);
    assert_eq!(market.symbols(), ["SYN0001", "SYN0002", "SYN0003"]);
    let ids: Vec<InstrumentId> = market
        .snapshots()
        .iter()
        .map(MarketEvent::instrument)
        .collect();
    let one = InstrumentId(1);
    let (two, three) = (InstrumentId(2), InstrumentId(3));
    assert_eq!(
        ids,
        [one, one, two, two, three, three],
        "a book, then its top"
    );
}

#[test]
fn the_same_seed_gives_the_same_market() {
    let run = |seed| {
        let mut market = SyntheticMarket::new(10, seed);
        let mut events = market.snapshots();
        for _ in 0..200 {
            events.extend(market.step(T));
        }
        events
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7), run(8));
}

#[test]
fn an_empty_market_makes_nothing() {
    let mut market = SyntheticMarket::new(0, 1);
    assert!(market.symbols().is_empty());
    assert!(market.snapshots().is_empty());
    assert!(market.step(T).is_empty());
}

/// Replays every event into real order books and checks them after each.
fn replay(market: &mut SyntheticMarket, steps: usize) -> [usize; 4] {
    let mut books: Vec<OrderBook> = market
        .snapshots()
        .iter()
        .filter_map(|e| match e {
            MarketEvent::Snapshot(s) => Some(OrderBook::from_snapshot(s)),
            _ => None,
        })
        .collect();
    let index = |id: InstrumentId| usize::try_from(id.0).expect("small") - 1;
    let mut kinds = [0; 4];
    let mut last_trade = 0;
    for _ in 0..steps {
        for event in market.step(T) {
            let book = &mut books[index(event.instrument())];
            match event {
                MarketEvent::Trade(trade) => {
                    kinds[0] += 1;
                    assert_eq!(trade.id, last_trade + 1, "trade numbers go up by one");
                    last_trade = trade.id;
                    let best = match trade.taker {
                        Side::Buy => book.best_ask(),
                        Side::Sell => book.best_bid(),
                    };
                    assert_eq!(Some(trade.price), best.map(|l| l.price), "at the touch");
                }
                MarketEvent::Delta(delta) => {
                    // A move touches both sides; new quantities touch one.
                    let moved = !delta.bids.is_empty() && !delta.asks.is_empty();
                    kinds[if moved { 3 } else { 1 }] += 1;
                    assert_eq!(book.apply(&delta), Ok(DeltaOutcome::Applied));
                    assert_eq!(book.depth(), (DEPTH, DEPTH), "{delta:?}");
                }
                MarketEvent::Top(top) => {
                    kinds[2] += 1;
                    assert_eq!(book.best_bid(), Some(top.bid));
                    assert_eq!(book.best_ask(), Some(top.ask));
                    assert!(top.bid.price < top.ask.price);
                    assert_eq!(top.update_id, book.update_id());
                }
                MarketEvent::Snapshot(_) => panic!("snapshots come only at the start"),
            }
        }
    }
    kinds
}

#[test]
fn every_book_change_keeps_a_true_book() {
    let mut market = SyntheticMarket::new(5, 42);
    let [trades, requotes, tops, moves] = replay(&mut market, 20_000);
    assert_eq!(
        requotes + moves,
        tops,
        "each change comes with its best prices"
    );
    assert_eq!(trades + requotes + moves, 20_000);
    assert!((5_500..6_500).contains(&trades), "3 in 10: {trades}");
    assert!((9_500..10_500).contains(&requotes), "5 in 10: {requotes}");
    assert!((3_500..4_500).contains(&moves), "2 in 10: {moves}");
}

#[test]
fn a_book_at_the_floor_always_moves_up() {
    let floor = ticks_away(DEPTH) + 1;
    let mut book = Book {
        id: InstrumentId(1),
        best_bid: floor,
        bids: (0..DEPTH).map(|_| 5).collect(),
        asks: (0..DEPTH).map(|_| 5).collect(),
        update_id: 1,
    };
    let mut rng = Rng(3);
    for _ in 0..50 {
        book.best_bid = floor;
        book.shift(&mut rng, T);
        assert_eq!(book.best_bid, floor + 1);
    }
    let lowest = book.bid_levels().last().map(|l| l.price);
    assert_eq!(lowest, Some(price(floor + 1 - ticks_away(DEPTH) + 1)));
    assert!(lowest.is_some_and(|p| p > price(0)));
}

#[test]
fn the_generator_is_splitmix64() {
    // The published SplitMix64 outputs for seed 0.
    let mut rng = Rng(0);
    assert_eq!(rng.next(), 0xe220_a839_7b1d_cdaf);
    assert_eq!(rng.next(), 0x6e78_9e6a_a1b9_65f4);
    assert_eq!(rng.next(), 0x06c4_5d18_8009_454f);
    // Odd, even, odd: tails, heads, tails.
    let mut rng = Rng(0);
    assert_eq!([rng.coin(), rng.coin(), rng.coin()], [false, true, false]);
}

#[test]
fn draws_stay_in_range_and_cover_it() {
    let mut rng = Rng(9);
    let mut indexes = [0; 7];
    let mut lots = std::collections::BTreeSet::new();
    let mut heads = 0;
    for _ in 0..10_000 {
        let lot = rng.lots();
        assert!((1..=1000).contains(&lot));
        lots.insert(lot);
        indexes[rng.index(7)] += 1;
        heads += usize::from(rng.coin());
    }
    assert!(
        indexes.iter().all(|n| *n > 1_000),
        "every index comes up: {indexes:?}"
    );
    assert!(lots.len() > 900, "most quantities come up: {}", lots.len());
    assert!((4_500..5_500).contains(&heads), "a fair coin: {heads}");
}
