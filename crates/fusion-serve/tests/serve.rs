//! A running server and a real WebSocket client.

use std::time::Duration;

use fusion_domain::{BookSnapshot, InstrumentId, Level, MarketEvent, TopOfBook};
use fusion_protocol::v1::{
    ClientMessage, CreateViewport, LoadLayout, SaveLayout, ServerMessage, ViewportSpec, ack,
    client_message, server_message,
};
use fusion_protocol::{Cell, decode, encode};
use fusion_serve::{MemoryLayouts, ServeConfig, serve};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const WAIT: Duration = Duration::from_secs(3);
const BTC: InstrumentId = InstrumentId(1);

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A server for BTCUSDT and ETHUSDT; market events go in on the sender.
async fn start() -> (
    String,
    mpsc::Sender<MarketEvent>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = format!("ws://{}", listener.local_addr().expect("an address"));
    let (tx, rx) = mpsc::channel(64);
    let symbols = ["BTCUSDT".to_owned(), "ETHUSDT".to_owned()];
    let running = tokio::spawn(async move {
        serve(
            listener,
            &symbols,
            rx,
            MemoryLayouts::default(),
            ServeConfig::default(),
        )
        .await;
    });
    (address, tx, running)
}

async fn connect(address: &str) -> Client {
    tokio_tungstenite::connect_async(address)
        .await
        .expect("connects")
        .0
}

async fn send(client: &mut Client, message: ClientMessage) {
    client
        .send(Message::Binary(encode(&message).into()))
        .await
        .expect("sent");
}

async fn next(client: &mut Client) -> server_message::Body {
    let frame = tokio::time::timeout(WAIT, client.next())
        .await
        .expect("a frame in time")
        .expect("a frame")
        .expect("no error");
    let Message::Binary(bytes) = frame else {
        panic!("not binary: {frame:?}");
    };
    decode::<ServerMessage>(&bytes)
        .expect("decodes")
        .body
        .expect("a body")
}

fn create(request: u64, viewport: u32, table: &str) -> ClientMessage {
    ClientMessage {
        request,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport,
            spec: Some(ViewportSpec {
                table: table.to_owned(),
                ..ViewportSpec::default()
            }),
        })),
    }
}

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn cell(value: &fusion_protocol::v1::Value) -> Cell {
    Cell::try_from(value).expect("fits")
}

#[tokio::test]
async fn a_client_gets_a_snapshot_then_only_changes() {
    let (address, market, _server) = start().await;
    let mut client = connect(&address).await;
    send(&mut client, create(1, 7, "prices")).await;
    assert!(matches!(next(&mut client).await, server_message::Body::Ack(a) if a.request == 1));
    let server_message::Body::Snapshot(snapshot) = next(&mut client).await else {
        panic!("a snapshot");
    };
    assert_eq!(snapshot.viewport, 7);
    assert_eq!(snapshot.rows.len(), 2);
    let bid = snapshot
        .columns
        .iter()
        .position(|c| c == "bid")
        .expect("a bid column");

    market
        .send(MarketEvent::Top(TopOfBook {
            instrument: BTC,
            update_id: 1,
            bid: level("100.5", "2"),
            ask: level("101", "3"),
        }))
        .await
        .expect("sent");
    let server_message::Body::Delta(delta) = next(&mut client).await else {
        panic!("a delta");
    };
    assert_eq!(delta.updated.len(), 1);
    let update = &delta.updated[0];
    assert_eq!(update.key, "BTCUSDT");
    let at = update
        .columns
        .iter()
        .position(|c| *c as usize == bid)
        .expect("the bid changed");
    assert_eq!(
        cell(&update.values[at]),
        Cell::Decimal("100.5".parse().expect("p"))
    );
}

#[tokio::test]
async fn a_book_table_shows_the_levels() {
    let (address, market, _server) = start().await;
    market
        .send(MarketEvent::Snapshot(BookSnapshot {
            instrument: BTC,
            update_id: 10,
            bids: vec![level("100", "1")],
            asks: vec![level("101", "2")],
        }))
        .await
        .expect("sent");
    let mut client = connect(&address).await;
    send(&mut client, create(1, 1, "book/BTCUSDT")).await;
    next(&mut client).await;
    let server_message::Body::Snapshot(snapshot) = next(&mut client).await else {
        panic!("a snapshot");
    };
    assert_eq!(snapshot.rows.len(), 40);
    let best_ask = snapshot.rows.iter().find(|r| r.key == "a00").expect("a00");
    assert_eq!(
        cell(&best_ask.cells[2]),
        Cell::Decimal("101".parse().expect("p"))
    );
}

#[tokio::test]
async fn bad_requests_and_bad_frames_get_errors() {
    let (address, _market, _server) = start().await;
    let mut client = connect(&address).await;
    send(&mut client, create(1, 1, "nope")).await;
    assert!(
        matches!(next(&mut client).await, server_message::Body::Error(e) if e.message == "there is no table nope")
    );
    client
        .send(Message::Binary(vec![0xff, 0xff].into()))
        .await
        .expect("sent");
    assert!(
        matches!(next(&mut client).await, server_message::Body::Error(e) if e.message == "not a message")
    );
    client.send(Message::text("hello")).await.expect("sent");
    assert!(matches!(
        next(&mut client).await,
        server_message::Body::Error(_)
    ));
}

#[tokio::test]
async fn layouts_are_saved_and_loaded() {
    let (address, _market, _server) = start().await;
    let mut client = connect(&address).await;
    let save = ClientMessage {
        request: 1,
        body: Some(client_message::Body::SaveLayout(SaveLayout {
            name: "main".into(),
            json: r#"{"schema_version":2}"#.into(),
        })),
    };
    send(&mut client, save).await;
    assert!(matches!(next(&mut client).await, server_message::Body::Ack(a) if a.request == 1));
    let load = |request, name: &str| ClientMessage {
        request,
        body: Some(client_message::Body::LoadLayout(LoadLayout {
            name: name.into(),
        })),
    };
    send(&mut client, load(2, "main")).await;
    let server_message::Body::Ack(answer) = next(&mut client).await else {
        panic!("an ack");
    };
    assert_eq!(
        answer.result,
        Some(ack::Result::LayoutJson(r#"{"schema_version":2}"#.into()))
    );
    send(&mut client, load(3, "other")).await;
    assert!(
        matches!(next(&mut client).await, server_message::Body::Error(e) if e.message == "there is no layout other")
    );
}

#[tokio::test]
async fn a_client_that_leaves_does_not_disturb_the_others() {
    let (address, _market, _server) = start().await;
    let mut leaving = connect(&address).await;
    send(&mut leaving, create(1, 1, "prices")).await;
    next(&mut leaving).await;
    leaving.close(None).await.expect("closed");
    drop(leaving);
    let mut staying = connect(&address).await;
    send(&mut staying, create(1, 1, "prices")).await;
    assert!(matches!(
        next(&mut staying).await,
        server_message::Body::Ack(_)
    ));
    assert!(matches!(
        next(&mut staying).await,
        server_message::Body::Snapshot(_)
    ));
}

#[tokio::test]
async fn the_server_stops_when_the_market_closes() {
    let (_address, market, server) = start().await;
    drop(market);
    tokio::time::timeout(WAIT, server)
        .await
        .expect("stops in time")
        .expect("no panic");
}

#[tokio::test]
async fn a_connection_that_is_not_a_websocket_is_dropped() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (address, _market, _server) = start().await;
    let host = address.trim_start_matches("ws://");
    let mut raw = TcpStream::connect(host).await.expect("connects");
    raw.write_all(b"hello, not http\r\n\r\n")
        .await
        .expect("written");
    let mut reply = Vec::new();
    // The server hangs up; the read ends.
    let read = tokio::time::timeout(WAIT, raw.read_to_end(&mut reply)).await;
    assert!(read.is_ok(), "the server hung up");
    // The server still serves others.
    let mut client = connect(&address).await;
    send(&mut client, create(1, 1, "prices")).await;
    assert!(matches!(
        next(&mut client).await,
        server_message::Body::Ack(_)
    ));
}

#[tokio::test]
async fn two_clients_at_once_each_get_their_own_answers() {
    let (address, _market, _server) = start().await;
    let mut first = connect(&address).await;
    let mut second = connect(&address).await;
    send(&mut first, create(11, 1, "prices")).await;
    send(&mut second, create(22, 1, "prices")).await;
    assert!(matches!(next(&mut first).await, server_message::Body::Ack(a) if a.request == 11));
    assert!(matches!(next(&mut second).await, server_message::Body::Ack(a) if a.request == 22));
    assert!(matches!(
        next(&mut first).await,
        server_message::Body::Snapshot(_)
    ));
    assert!(matches!(
        next(&mut second).await,
        server_message::Body::Snapshot(_)
    ));
}
