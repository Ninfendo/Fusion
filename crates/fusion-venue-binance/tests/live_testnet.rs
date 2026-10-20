//! The live connection against the real Spot Testnet. It needs the
//! internet, so CI skips it; run it by hand:
//!
//! `cargo test -p fusion-venue-binance --test live_testnet -- --ignored --nocapture`

use std::time::Duration;

use fusion_domain::MarketEvent;
use fusion_venue_binance::live::{self, FeedItem, HttpFetch, LiveConfig};
use fusion_venue_binance::{Endpoint, SymbolTable};
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "needs the internet and the Binance Spot Testnet"]
async fn the_testnet_streams_and_books_sync() {
    let symbols = SymbolTable::new(["BTCUSDT", "ETHUSDT"]);
    let config = LiveConfig::new(Endpoint::Testnet, symbols);
    let (out, mut items) = mpsc::channel(4096);
    tokio::spawn(live::run(config, HttpFetch::new().expect("a client"), out));

    let (mut snapshots, mut deltas, mut trades, mut tops, mut notes) = (0, 0, 0, 0, Vec::new());
    let _ = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(item) = items.recv().await {
            match item {
                FeedItem::Event(MarketEvent::Snapshot(_)) => snapshots += 1,
                FeedItem::Event(MarketEvent::Delta(_)) => deltas += 1,
                FeedItem::Event(MarketEvent::Trade(_)) => trades += 1,
                FeedItem::Event(MarketEvent::Top(_)) => tops += 1,
                other => notes.push(other),
            }
        }
    })
    .await;
    println!("snapshots {snapshots}, deltas {deltas}, trades {trades}, tops {tops}");
    println!("notes {notes:?}");
    assert_eq!(snapshots, 2, "both books synced once");
    assert!(deltas > 0 && tops > 0);
}
