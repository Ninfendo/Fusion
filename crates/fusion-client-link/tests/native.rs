//! The desktop link against a real view server on this machine.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use fusion_client_link::{Link, LinkEvent};
use fusion_domain::{InstrumentId, Level, MarketEvent, TopOfBook};
use fusion_protocol::v1::{
    ClientMessage, CreateViewport, ViewportSpec, client_message, server_message,
};
use fusion_serve::{MemoryLayouts, ServeConfig, serve};
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(15);

/// A free local port: bound, then let go.
fn free_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.local_addr().expect("an address").to_string()
}

/// Runs a view server for BTCUSDT on `address` on a thread of its own, until
/// the returned sender is dropped.
fn start_server(address: &str) -> (mpsc::Sender<MarketEvent>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(16);
    let address = address.to_owned();
    let server = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(&address)
                .await
                .expect("binds");
            let symbols = ["BTCUSDT".to_owned()];
            serve(
                listener,
                &symbols,
                rx,
                MemoryLayouts::default(),
                ServeConfig::default(),
            )
            .await;
        });
    });
    (tx, server)
}

/// Polls the link until `want` matches an event.
fn until(link: &mut Link, want: impl Fn(&LinkEvent) -> bool) -> Vec<LinkEvent> {
    let start = Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < WAIT {
        let events = link.poll();
        let hit = events.iter().any(&want);
        seen.extend(events);
        if hit {
            return seen;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("waited in vain; saw {seen:?}");
}

fn open_prices() -> ClientMessage {
    ClientMessage {
        request: 1,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport: 1,
            spec: Some(ViewportSpec {
                table: "prices".into(),
                ..ViewportSpec::default()
            }),
        })),
    }
}

fn is_body(event: &LinkEvent, test: impl Fn(&server_message::Body) -> bool) -> bool {
    matches!(event, LinkEvent::Message(m) if m.body.as_ref().is_some_and(&test))
}

#[test]
fn the_link_waits_for_the_server_and_follows_it() {
    let address = free_address();
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    let mut link = Link::connect(&format!("ws://{address}"), move || {
        woken.fetch_add(1, Ordering::SeqCst);
    })
    .expect("starts");

    // Nobody listens yet: the link says so, and tries again.
    until(&mut link, |e| matches!(e, LinkEvent::Disconnected(_)));
    link.send(&open_prices());
    let (market, server) = start_server(&address);
    until(&mut link, |e| *e == LinkEvent::Connected);

    link.send(&open_prices());
    until(&mut link, |e| {
        is_body(e, |b| matches!(b, server_message::Body::Snapshot(_)))
    });
    market
        .blocking_send(MarketEvent::Top(TopOfBook {
            instrument: InstrumentId(1),
            update_id: 1,
            bid: Level::new("1".parse().expect("p"), "1".parse().expect("q")),
            ask: Level::new("2".parse().expect("p"), "1".parse().expect("q")),
        }))
        .expect("sent");
    until(&mut link, |e| {
        is_body(e, |b| matches!(b, server_message::Body::Delta(_)))
    });
    assert!(wakes.load(Ordering::SeqCst) >= 4, "woken for each event");

    // The server stops: the link says so.
    drop(market);
    server.join().expect("the server stopped");
    until(&mut link, |e| matches!(e, LinkEvent::Disconnected(_)));
}

#[test]
fn a_dropped_link_stops_its_thread() {
    let address = free_address();
    let (market, server) = start_server(&address);
    let mut link = Link::connect(&format!("ws://{address}"), || {}).expect("starts");
    until(&mut link, |e| *e == LinkEvent::Connected);
    drop(link);
    drop(market);
    server.join().expect("the server stopped");
}
