//! The live feed against a real view server: prices, a book, and the
//! status line.

use std::collections::BTreeSet;
use std::thread;
use std::time::{Duration, Instant};

use fusion_client_core::{FeedStatus, MarketData, MarketFeed, Price};
use fusion_client_link::LiveFeed;
use fusion_domain::{BookSnapshot, InstrumentId, Level, MarketEvent, TopOfBook};

mod common;

use common::{free_address, start_server};

const BTC: InstrumentId = InstrumentId(1);

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

/// Updates the feed until `done` holds.
fn until(feed: &mut LiveFeed, what: &str, done: impl Fn(&LiveFeed) -> bool) {
    let start = Instant::now();
    loop {
        feed.update(0.0);
        if done(feed) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "waited in vain for {what}; status: {}",
            feed.status()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_feed_shows_the_server_s_prices_and_books() {
    let address = free_address();
    let (market, server) = start_server(&address);
    let mut feed = LiveFeed::connect(&format!("ws://{address}"), || {}).expect("starts");
    assert_eq!(feed.status(), FeedStatus::Connecting);
    until(&mut feed, "a connection", |f| {
        f.status() == FeedStatus::Live
    });
    until(&mut feed, "the symbols", |f| f.symbols() == ["BTCUSDT"]);
    assert_eq!(feed.quotes(), [], "no bid and ask yet");

    market
        .blocking_send(MarketEvent::Top(TopOfBook {
            instrument: BTC,
            update_id: 1,
            bid: level("100", "1"),
            ask: level("101", "2"),
        }))
        .expect("sent");
    until(&mut feed, "a quote", |f| {
        f.quotes()
            .first()
            .is_some_and(|q| q.bid == Price::new("100".parse().expect("p")))
    });

    assert_eq!(feed.book("BTCUSDT", 5), None, "no book wanted yet");
    feed.want_books(&BTreeSet::from(["BTCUSDT".to_owned()]));
    market
        .blocking_send(MarketEvent::Snapshot(BookSnapshot {
            instrument: BTC,
            update_id: 2,
            bids: vec![level("100", "1"), level("99", "3")],
            asks: vec![level("101", "2")],
        }))
        .expect("sent");
    until(&mut feed, "a book", |f| {
        f.book("BTCUSDT", 5)
            .is_some_and(|b| b.bids.len() == 2 && b.asks.len() == 1)
    });
    feed.want_books(&BTreeSet::new());
    assert_eq!(feed.book("BTCUSDT", 5), None, "dropped");

    drop(market);
    server.join().expect("the server stopped");
    until(&mut feed, "the loss", |f| {
        matches!(f.status(), FeedStatus::Away(_))
    });
    assert!(feed.status().to_string().starts_with("Not connected: "));
}
