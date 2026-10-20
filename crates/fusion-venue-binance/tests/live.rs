//! The live connection against a fake Binance on this machine: a WebSocket
//! server the test scripts, and REST replies the test gives.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fusion_domain::{InstrumentId, MarketEvent};
use fusion_venue_binance::live::{
    self, Endpoints, FeedItem, Fetch, HttpFetch, LinkNote, LiveConfig,
};
use fusion_venue_binance::{Note, SymbolTable};
use futures_util::SinkExt as _;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

const WAIT: Duration = Duration::from_secs(3);
const BTC: InstrumentId = InstrumentId(1);

/// One connection to the fake server: the test sends text, or `None` to
/// close it.
type ServerConn = mpsc::UnboundedSender<Option<String>>;

/// A WebSocket server on a free local port. Each new connection arrives
/// on the returned receiver.
async fn fake_stream() -> (String, mpsc::UnboundedReceiver<ServerConn>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = format!("ws://{}", listener.local_addr().expect("an address"));
    let (conns_tx, conns_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else {
                continue;
            };
            let (tx, rx) = mpsc::unbounded_channel::<Option<String>>();
            if conns_tx.send(tx).is_err() {
                return;
            }
            tokio::spawn(serve(ws, rx));
        }
    });
    (address, conns_rx)
}

/// Sends what the test gives on one server connection, until `None`.
async fn serve<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    mut rx: mpsc::UnboundedReceiver<Option<String>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(Some(text)) = rx.recv().await {
        let sent = match text.as_str() {
            // Hang up without a goodbye.
            "!drop" => return,
            "!bye" => {
                let frame = CloseFrame {
                    code: CloseCode::Away,
                    reason: "bye".into(),
                };
                ws.close(Some(frame)).await
            }
            _ => ws.send(WsMessage::text(text)).await,
        };
        if sent.is_err() {
            return;
        }
    }
    // None, or the test dropped the connection: close it.
    let _closed = ws.close(None).await;
}

/// Replies waiting, by a part of the address they answer.
type Replies = BTreeMap<String, VecDeque<Result<String, String>>>;

/// REST replies the test lines up, per address part: the first reply that
/// matches a requested address is used once.
#[derive(Clone, Default)]
struct FakeRest {
    replies: Arc<Mutex<Replies>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl FakeRest {
    fn reply(&self, part: &str, reply: Result<String, String>) {
        self.replies
            .lock()
            .expect("unlocked")
            .entry(part.to_owned())
            .or_default()
            .push_back(reply);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("unlocked").clone()
    }
}

impl Fetch for FakeRest {
    fn get(&self, url: String) -> impl Future<Output = Result<String, String>> + Send {
        self.asked.lock().expect("unlocked").push(url.clone());
        let reply = self
            .replies
            .lock()
            .expect("unlocked")
            .iter_mut()
            .find(|(part, queue)| url.contains(part.as_str()) && !queue.is_empty())
            .and_then(|(_, queue)| queue.pop_front())
            .unwrap_or_else(|| Err(format!("no reply for {url}")));
        async move { reply }
    }
}

fn config(stream: &str) -> LiveConfig {
    LiveConfig {
        endpoints: Endpoints {
            stream: stream.to_owned(),
            rest: "http://rest".to_owned(),
        },
        symbols: SymbolTable::new(["BTCUSDT"]),
        rotate_after: Duration::from_secs(3600),
        stall_after: Duration::from_secs(3600),
        overlap_limit: Duration::from_secs(3600),
        first_retry: Duration::from_millis(20),
        check_every: Duration::from_secs(3600),
    }
}

fn depth(first: u64, last: u64) -> String {
    format!(
        r#"{{"stream":"btcusdt@depth@100ms","data":{{"e":"depthUpdate","E":1,"s":"BTCUSDT","U":{first},"u":{last},"b":[["100.{last}","1"]],"a":[]}}}}"#
    )
}

fn agg_trade(id: u64) -> String {
    format!(
        r#"{{"stream":"btcusdt@aggTrade","data":{{"e":"aggTrade","E":1,"s":"BTCUSDT","a":{id},"p":"100","q":"1","f":1,"l":1,"T":1,"m":false,"M":true}}}}"#
    )
}

fn snapshot(update_id: u64) -> String {
    format!(r#"{{"lastUpdateId":{update_id},"bids":[["99","1"]],"asks":[["101","1"]]}}"#)
}

fn rest_trades(ids: &[u64]) -> String {
    let items: Vec<String> = ids
        .iter()
        .map(|id| format!(r#"{{"a":{id},"p":"100","q":"1","f":1,"l":1,"T":1,"m":true,"M":true}}"#))
        .collect();
    format!("[{}]", items.join(","))
}

/// Starts the live connection; its items arrive on the receiver.
fn start(config: LiveConfig, rest: FakeRest) -> mpsc::Receiver<FeedItem> {
    let (out, items) = mpsc::channel(1024);
    tokio::spawn(live::run(config, rest, out));
    items
}

/// Reads items until one matches `want`, and returns every item read.
async fn until(
    items: &mut mpsc::Receiver<FeedItem>,
    want: impl Fn(&FeedItem) -> bool,
) -> Vec<FeedItem> {
    until_within(WAIT, items, want).await
}

/// [`until`] with its own time limit.
async fn until_within(
    wait: Duration,

    items: &mut mpsc::Receiver<FeedItem>,
    want: impl Fn(&FeedItem) -> bool,
) -> Vec<FeedItem> {
    let mut seen = Vec::new();
    let found = tokio::time::timeout(wait, async {
        while let Some(item) = items.recv().await {
            let hit = want(&item);
            seen.push(item);
            if hit {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(found, Ok(true), "waited in vain; saw {seen:?}");
    seen
}

async fn next_conn(conns: &mut mpsc::UnboundedReceiver<ServerConn>) -> ServerConn {
    tokio::time::timeout(WAIT, conns.recv())
        .await
        .expect("a connection in time")
        .expect("a connection")
}

fn send(conn: &ServerConn, text: String) {
    conn.send(Some(text)).expect("the connection is open");
}

fn delta_ranges(items: &[FeedItem]) -> Vec<(u64, u64)> {
    items
        .iter()
        .filter_map(|item| match item {
            FeedItem::Event(MarketEvent::Delta(d)) => Some((d.first_update, d.last_update)),
            _ => None,
        })
        .collect()
}

fn is_delta_to(last: u64) -> impl Fn(&FeedItem) -> bool {
    move |item| matches!(item, FeedItem::Event(MarketEvent::Delta(d)) if d.last_update == last)
}

fn notes(items: &[FeedItem]) -> Vec<&Note> {
    items
        .iter()
        .filter_map(|item| match item {
            FeedItem::Note(note) => Some(note),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_book_syncs_from_a_snapshot_and_the_stream() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth?symbol=BTCUSDT", Ok(snapshot(10)));
    let mut items = start(config(&stream), rest.clone());
    let conn = next_conn(&mut conns).await;
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Opened(1)))
    })
    .await;
    send(&conn, depth(9, 11));
    send(&conn, depth(12, 12));
    let seen = until(&mut items, is_delta_to(12)).await;
    assert!(
        seen.iter()
            .any(|i| matches!(i, FeedItem::Event(MarketEvent::Snapshot(s)) if s.update_id == 10))
    );
    assert_eq!(delta_ranges(&seen), [(9, 11), (12, 12)]);
    assert_eq!(
        rest.asked(),
        ["http://rest/api/v3/depth?symbol=BTCUSDT&limit=1000"]
    );
}

#[tokio::test]
async fn a_lost_connection_is_opened_again_and_the_book_resyncs() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Ok(snapshot(10)));
    rest.reply("depth", Ok(snapshot(20)));
    let mut items = start(config(&stream), rest);
    let first = next_conn(&mut conns).await;
    send(&first, depth(10, 11));
    until(&mut items, is_delta_to(11)).await;
    first.send(None).expect("closes");
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Closed(1, _)))
    })
    .await;

    let second = next_conn(&mut conns).await;
    send(&second, depth(20, 21));
    let seen = until(&mut items, is_delta_to(21)).await;
    assert!(matches!(notes(&seen).as_slice(), [Note::BookGap(BTC, _)]));
    assert!(
        seen.iter()
            .any(|i| matches!(i, FeedItem::Link(LinkNote::Opened(2))))
    );
}

#[tokio::test]
async fn the_next_connection_takes_over_without_a_gap() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Ok(snapshot(10)));
    let mut items = start(
        LiveConfig {
            rotate_after: Duration::from_millis(300),
            ..config(&stream)
        },
        rest,
    );
    let old = next_conn(&mut conns).await;
    send(&old, depth(10, 11));
    send(&old, agg_trade(500));
    until(&mut items, is_delta_to(11)).await;

    // The new connection starts ahead of the old one: its first change
    // (13-14) does not fit yet, so the old one keeps the book.
    let new = next_conn(&mut conns).await;
    send(&new, depth(13, 14));
    send(&new, agg_trade(502));
    send(&old, depth(12, 12));
    send(&old, agg_trade(501));
    send(&old, depth(13, 14));
    send(&old, agg_trade(502));
    // Now the new connection's data fits, and it takes over.
    send(&new, depth(15, 15));
    send(&new, agg_trade(503));
    let seen = until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { from: 1, to: 2 }))
    })
    .await;
    send(&new, depth(16, 16));
    let rest_of_it = until(&mut items, is_delta_to(16)).await;

    let all: Vec<FeedItem> = seen.into_iter().chain(rest_of_it).collect();
    assert_eq!(delta_ranges(&all), [(12, 12), (13, 14), (15, 15), (16, 16)]);
    let trades: Vec<u64> = all
        .iter()
        .filter_map(|i| match i {
            FeedItem::Event(MarketEvent::Trade(t)) => Some(t.id),
            _ => None,
        })
        .collect();
    assert_eq!(trades, [501, 502, 503]);
    assert!(
        notes(&all).is_empty(),
        "no gap and no missed trade: {all:?}"
    );
}

#[tokio::test]
async fn a_server_shutdown_opens_the_next_connection_at_once() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(config(&stream), FakeRest::default());
    let old = next_conn(&mut conns).await;
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let _new = next_conn(&mut conns).await;
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Opened(2)))
    })
    .await;
}

#[tokio::test]
async fn an_overlap_ends_after_its_limit_caught_up_or_not() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(
        LiveConfig {
            overlap_limit: Duration::from_millis(200),
            ..config(&stream)
        },
        FakeRest::default(),
    );
    let old = next_conn(&mut conns).await;
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let _new = next_conn(&mut conns).await;
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { from: 1, to: 2 }))
    })
    .await;
}

#[tokio::test]
async fn a_silent_connection_is_replaced() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(
        LiveConfig {
            stall_after: Duration::from_millis(200),
            ..config(&stream)
        },
        FakeRest::default(),
    );
    let _quiet = next_conn(&mut conns).await;
    let seen = until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Opened(2)))
    })
    .await;
    assert!(seen.iter().any(
        |i| matches!(i, FeedItem::Link(LinkNote::Closed(1, why)) if why == "silent for too long")
    ));
}

#[tokio::test]
async fn missed_trades_are_fetched() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("aggTrades", Ok(rest_trades(&[101, 102, 103])));
    let mut items = start(config(&stream), rest.clone());
    let conn = next_conn(&mut conns).await;
    send(&conn, agg_trade(100));
    send(&conn, agg_trade(103));
    let seen = until(
        &mut items,
        |i| matches!(i, FeedItem::Event(MarketEvent::Trade(t)) if t.id == 102),
    )
    .await;
    assert!(notes(&seen).contains(&&Note::MissedTrades(BTC, 101..103)));
    assert_eq!(
        rest.asked(),
        ["http://rest/api/v3/aggTrades?symbol=BTCUSDT&fromId=101&limit=1000"]
    );
}

#[tokio::test]
async fn a_failed_fetch_is_tried_again() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Err("busy".to_owned()));
    rest.reply("depth", Ok("not json".to_owned()));
    rest.reply("depth", Ok(snapshot(10)));
    let mut items = start(config(&stream), rest.clone());
    let conn = next_conn(&mut conns).await;
    send(&conn, depth(10, 11));
    let seen = until(&mut items, is_delta_to(11)).await;
    let failures: Vec<&String> = seen
        .iter()
        .filter_map(|i| match i {
            FeedItem::Link(LinkNote::FetchFailed { error, .. }) => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0], "busy");
    let attempts: Vec<u32> = seen
        .iter()
        .filter_map(|i| match i {
            FeedItem::Link(LinkNote::FetchFailed { attempt, .. }) => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, [0, 1]);
    assert_eq!(rest.asked().len(), 3);
}

#[tokio::test]
async fn a_bad_message_is_reported_and_reading_goes_on() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(config(&stream), FakeRest::default());
    let conn = next_conn(&mut conns).await;
    send(&conn, "{ nope".to_owned());
    send(&conn, agg_trade(1));
    let seen = until(&mut items, |i| {
        matches!(i, FeedItem::Event(MarketEvent::Trade(_)))
    })
    .await;
    assert!(
        seen.iter()
            .any(|i| matches!(i, FeedItem::Link(LinkNote::BadMessage(_))))
    );
}

#[tokio::test]
async fn a_server_that_is_not_there_is_tried_again() {
    // Bind a port, then free it: nothing listens there.
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        format!("ws://{}", listener.local_addr().expect("an address"))
    };
    let mut items = start(config(&address), FakeRest::default());
    // On Windows a refused local connection takes about 2 seconds to fail.
    until_within(Duration::from_secs(15), &mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Closed(2, _)))
    })
    .await;
}

#[tokio::test]
async fn dropping_the_receiver_stops_the_feed() {
    let (stream, mut conns) = fake_stream().await;
    let (out, items) = mpsc::channel(8);
    let running = tokio::spawn(live::run(config(&stream), FakeRest::default(), out));
    let _conn = next_conn(&mut conns).await;
    drop(items);
    tokio::time::timeout(WAIT, running)
        .await
        .expect("stops in time")
        .expect("no panic");
}

// --- HttpFetch, against a tiny HTTP server ----------------------------------

async fn http_server(status: &'static str, body: &'static str) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let mut request = [0u8; 1024];
            let _read = tcp.read(&mut request).await;
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _written = tcp.write_all(reply.as_bytes()).await;
        }
    });
    address
}

#[tokio::test]
async fn http_fetch_reads_a_body_and_reports_errors() {
    let fetch = HttpFetch::new().expect("a client");
    let ok = http_server("200 OK", "{}").await;
    assert_eq!(fetch.get(format!("{ok}/x")).await, Ok("{}".to_owned()));

    let refused = http_server("429 Too Many Requests", "slow down").await;
    let err = fetch
        .get(format!("{refused}/x"))
        .await
        .expect_err("refused");
    assert!(err.contains("429") && err.contains("slow down"), "{err}");

    let gone = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        format!("http://{}", listener.local_addr().expect("an address"))
    };
    assert!(fetch.get(gone).await.is_err());
}

#[tokio::test]
async fn the_reason_a_connection_ended_is_reported() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(config(&stream), FakeRest::default());
    let polite = next_conn(&mut conns).await;
    send(&polite, "!bye".to_owned());
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Closed(1, why)) if why == "closed by the server: 1001 bye")
    })
    .await;
    let rude = next_conn(&mut conns).await;
    send(&rude, "!drop".to_owned());
    let seen = until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Closed(2, _)))
    })
    .await;
    let why = seen.iter().find_map(|i| match i {
        FeedItem::Link(LinkNote::Closed(2, why)) => Some(why.clone()),
        _ => None,
    });
    assert!(why.is_some_and(|w| !w.starts_with("closed by the server")));
}

#[tokio::test]
async fn live_books_pass_on_their_fingerprints() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Ok(snapshot(10)));
    let mut items = start(
        LiveConfig {
            check_every: Duration::from_millis(100),
            symbols: SymbolTable::new(["BTCUSDT", "ETHUSDT"]),
            ..config(&stream)
        },
        rest,
    );
    let conn = next_conn(&mut conns).await;
    send(&conn, depth(10, 11));
    let seen = until(&mut items, |i| matches!(i, FeedItem::Check { .. })).await;
    let Some(FeedItem::Check {
        instrument,
        update_id,
        digest,
    }) = seen.last().cloned()
    else {
        panic!("a check");
    };
    assert_eq!(
        (instrument, update_id),
        (BTC, 11),
        "only the BTC book is live"
    );
    let mut book = fusion_domain::OrderBook::from_snapshot(
        &fusion_venue_binance::parse_snapshot(&snapshot(10), BTC).expect("reads"),
    );
    let MarketEvent::Delta(delta) = seen
        .iter()
        .find_map(|i| match i {
            FeedItem::Event(e @ MarketEvent::Delta(_)) => Some(e.clone()),
            _ => None,
        })
        .expect("a delta")
    else {
        panic!("a delta");
    };
    book.apply(&delta).expect("fits");
    assert_eq!(digest, book.digest());
}

/// Sends a text on the old server connection until it fails, which it does
/// once the client has hung up.
async fn hung_up(conn: &ServerConn) -> bool {
    tokio::time::timeout(WAIT, async {
        loop {
            if conn.send(Some(agg_trade(1))).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn the_old_connection_is_closed_after_the_new_one_took_over() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(
        LiveConfig {
            overlap_limit: Duration::from_millis(100),
            ..config(&stream)
        },
        FakeRest::default(),
    );
    let old = next_conn(&mut conns).await;
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let _new = next_conn(&mut conns).await;
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { .. }))
    })
    .await;
    assert!(hung_up(&old).await, "the old connection was closed");
}

#[tokio::test]
async fn an_old_connection_that_closes_hands_over_at_once() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(config(&stream), FakeRest::default());
    let old = next_conn(&mut conns).await;
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let _new = next_conn(&mut conns).await;
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Opened(2)))
    })
    .await;
    old.send(None).expect("closes");
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { from: 1, to: 2 }))
    })
    .await;
}

#[tokio::test]
async fn a_new_connection_that_fails_is_tried_again_after_a_wait() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(
        LiveConfig {
            first_retry: Duration::from_millis(400),
            ..config(&stream)
        },
        FakeRest::default(),
    );
    let old = next_conn(&mut conns).await;
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let failing = next_conn(&mut conns).await;
    failing.send(None).expect("closes");
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Closed(2, _)))
    })
    .await;
    let closed_at = tokio::time::Instant::now();
    let _third = next_conn(&mut conns).await;
    let waited = closed_at.elapsed();
    assert!(waited >= Duration::from_millis(300), "waited {waited:?}");
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::Opened(3)))
    })
    .await;
    old.send(None).expect("closes");
    until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { from: 1, to: 3 }))
    })
    .await;
}

#[tokio::test]
async fn the_wait_before_trying_again_doubles() {
    // A server that takes the connection and hangs up at once.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = format!("ws://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            drop(tcp);
        }
    });
    let mut items = start(
        LiveConfig {
            first_retry: Duration::from_millis(300),
            ..config(&address)
        },
        FakeRest::default(),
    );
    let mut closed = Vec::new();
    for n in 1..=3 {
        until(
            &mut items,
            |i| matches!(i, FeedItem::Link(LinkNote::Closed(c, _)) if *c == n),
        )
        .await;
        closed.push(tokio::time::Instant::now());
    }
    let first = closed[1] - closed[0];
    let second = closed[2] - closed[1];
    assert!(first >= Duration::from_millis(250), "first wait {first:?}");
    assert!(
        second >= first + Duration::from_millis(200),
        "{first:?} then {second:?}"
    );
}

#[tokio::test]
async fn nothing_rotates_or_hands_over_early() {
    let (stream, mut conns) = fake_stream().await;
    let mut items = start(config(&stream), FakeRest::default());
    let old = next_conn(&mut conns).await;
    // No second connection before rotate_after (an hour here).
    let early = tokio::time::timeout(Duration::from_millis(400), conns.recv()).await;
    assert!(early.is_err(), "no second connection yet");
    // An overlap keeps both connections until the new one has caught up,
    // or its limit (an hour here) passes.
    send(&old, r#"{"e":"serverShutdown","E":1}"#.to_owned());
    let _new = next_conn(&mut conns).await;
    let seen = tokio::time::timeout(Duration::from_millis(400), async {
        while let Some(item) = items.recv().await {
            if matches!(item, FeedItem::Link(LinkNote::TookOver { .. })) {
                return true;
            }
        }
        false
    })
    .await;
    assert!(!matches!(seen, Ok(true)), "no hand-over yet");
}

#[tokio::test]
async fn books_are_checked_no_more_often_than_asked() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Ok(snapshot(10)));
    // config() checks once an hour.
    let mut items = start(config(&stream), rest);
    let conn = next_conn(&mut conns).await;
    send(&conn, depth(10, 11));
    until(&mut items, is_delta_to(11)).await;
    let checks = tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(item) = items.recv().await {
            if matches!(item, FeedItem::Check { .. }) {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        !matches!(checks, Ok(true)),
        "no check before the hour is up"
    );
}

#[tokio::test]
async fn a_new_connection_ahead_of_the_old_one_takes_over_without_a_gap() {
    let (stream, mut conns) = fake_stream().await;
    let rest = FakeRest::default();
    rest.reply("depth", Ok(snapshot(10)));
    let mut items = start(
        LiveConfig {
            rotate_after: Duration::from_millis(300),
            ..config(&stream)
        },
        rest,
    );
    let old = next_conn(&mut conns).await;
    send(&old, depth(10, 11));
    send(&old, agg_trade(500));
    until(&mut items, is_delta_to(11)).await;
    let new = next_conn(&mut conns).await;
    // The new connection is ahead: all of its data arrives first.
    send(&new, depth(13, 14));
    send(&new, agg_trade(502));
    send(&new, depth(15, 15));
    send(&new, agg_trade(503));
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Then the old one catches up.
    send(&old, depth(12, 12));
    send(&old, agg_trade(501));
    let seen = until(&mut items, |i| {
        matches!(i, FeedItem::Link(LinkNote::TookOver { from: 1, to: 2 }))
    })
    .await;
    assert_eq!(delta_ranges(&seen), [(12, 12), (13, 14), (15, 15)]);
    let trades: Vec<u64> = seen
        .iter()
        .filter_map(|i| match i {
            FeedItem::Event(MarketEvent::Trade(t)) => Some(t.id),
            _ => None,
        })
        .collect();
    assert_eq!(trades, [501, 502, 503]);
    assert!(
        notes(&seen).is_empty(),
        "no gap and no missed trade: {seen:?}"
    );
}
