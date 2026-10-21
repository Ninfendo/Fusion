//! One layout kept on the view server: a real server for the normal path,
//! fake ones for failures and lost connections.

use std::thread;
use std::time::Duration;

use fusion_client_link::{RemoteEvent, RemoteLayouts};
use fusion_protocol::v1::{
    Ack, ClientMessage, Error, ServerMessage, Snapshot, ack, client_message, server_message,
};
use fusion_protocol::{decode, encode};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

mod common;

use common::{free_address, start_server};

const WAIT: Duration = Duration::from_secs(10);

fn url(address: &str) -> String {
    format!("ws://{address}")
}

#[test]
fn a_layout_saved_by_one_client_loads_in_the_next() {
    let address = free_address();
    let (_market, _server) = start_server(&address);
    let mut first = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(
        first.wait_loaded(WAIT),
        Some(Ok(None)),
        "nothing stored yet"
    );
    first.save(r#"{"a":1}"#.into());
    assert_eq!(first.settle(WAIT), []);
    assert!(!first.pending());

    let mut second = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(
        second.wait_loaded(WAIT),
        Some(Ok(Some(r#"{"a":1}"#.to_owned())))
    );
    let mut other = RemoteLayouts::connect(&url(&address), "other", || {}).expect("starts");
    assert_eq!(other.wait_loaded(WAIT), Some(Ok(None)));
}

#[test]
fn a_save_made_before_the_server_starts_goes_out_when_it_does() {
    let address = free_address();
    let mut layouts = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(
        layouts.wait_loaded(Duration::from_millis(100)),
        None,
        "no server yet"
    );
    layouts.save(r#"{"early":true}"#.into());
    layouts.save(r#"{"early":"newest"}"#.into());
    assert!(layouts.pending());

    let (_market, _server) = start_server(&address);
    assert_eq!(layouts.settle(WAIT), [RemoteEvent::Loaded(Ok(None))]);
    assert!(!layouts.pending());
    let mut later = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(
        later.wait_loaded(WAIT),
        Some(Ok(Some(r#"{"early":"newest"}"#.to_owned()))),
        "only the newest was kept"
    );
}

type Socket = WebSocketStream<TcpStream>;

/// Runs `script` against each of `connections` clients in turn, on a
/// thread of its own.
fn fake_server<F>(
    connections: usize,
    script: impl Fn(usize, Socket) -> F + Send + 'static,
) -> (String, thread::JoinHandle<()>)
where
    F: Future<Output = ()>,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = listener.local_addr().expect("an address").to_string();
    listener.set_nonblocking(true).expect("non-blocking");
    let server = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async move {
            let listener = TcpListener::from_std(listener).expect("a listener");
            for n in 0..connections {
                let accepted = tokio::time::timeout(WAIT, listener.accept()).await;
                let (tcp, _) = accepted.expect("the client comes").expect("a client");
                let ws = tokio_tungstenite::accept_async(tcp)
                    .await
                    .expect("a WebSocket");
                script(n, ws).await;
            }
        });
    });
    (address, server)
}

/// The next request, or a failed test after [`WAIT`].
async fn read(ws: &mut Socket) -> ClientMessage {
    loop {
        let frame = tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("a message");
        if let Some(Ok(Message::Binary(bytes))) = frame {
            return decode(&bytes).expect("a client message");
        }
    }
}

async fn reply(ws: &mut Socket, body: server_message::Body) {
    let message = ServerMessage { body: Some(body) };
    ws.send(Message::Binary(encode(&message).into()))
        .await
        .expect("sent");
}

fn ack(request: u64, layout: Option<&str>) -> server_message::Body {
    server_message::Body::Ack(Ack {
        request,
        result: layout.map(|json| ack::Result::LayoutJson(json.to_owned())),
    })
}

#[test]
fn failures_are_reported_and_strangers_ignored() {
    let (address, server) = fake_server(1, |_, mut ws| async move {
        // Not about any request of this client: ignored.
        reply(&mut ws, ack(99, None)).await;
        reply(&mut ws, server_message::Body::Snapshot(Snapshot::default())).await;
        for _ in 0..2 {
            let request = read(&mut ws).await.request;
            let message = "the store is down".to_owned();
            let failed = Error {
                request,
                viewport: 0,
                message,
            };
            reply(&mut ws, server_message::Body::Error(failed)).await;
        }
    });
    let mut layouts = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(
        layouts.wait_loaded(WAIT),
        Some(Err("the store is down".to_owned()))
    );
    layouts.save("{}".into());
    assert_eq!(
        layouts.settle(WAIT),
        [RemoteEvent::SaveFailed("the store is down".to_owned())]
    );
    assert!(!layouts.pending(), "a refused save is not sent again");
    server.join().expect("the script ran");
}

#[test]
fn what_was_not_answered_is_asked_again_after_a_lost_connection() {
    let (address, server) = fake_server(3, |n, mut ws| async move {
        let message = read(&mut ws).await;
        match (n, message.body) {
            // The load: unanswered, then answered.
            (0, Some(client_message::Body::LoadLayout(_))) => {}
            (1, Some(client_message::Body::LoadLayout(_))) => {
                reply(&mut ws, ack(message.request, None)).await;
                // The save: unanswered, then answered.
                let save = read(&mut ws).await;
                assert!(matches!(
                    save.body,
                    Some(client_message::Body::SaveLayout(_))
                ));
            }
            (2, Some(client_message::Body::SaveLayout(s))) => {
                assert_eq!(s.json, r#"{"b":2}"#);
                reply(&mut ws, ack(message.request, None)).await;
            }
            other => panic!("connection {n} got {other:?}"),
        }
    });
    let mut layouts = RemoteLayouts::connect(&url(&address), "main", || {}).expect("starts");
    assert_eq!(layouts.wait_loaded(WAIT), Some(Ok(None)));
    layouts.save(r#"{"b":2}"#.into());
    assert_eq!(layouts.settle(WAIT), []);
    assert!(!layouts.pending());
    server.join().expect("the script ran");
}
