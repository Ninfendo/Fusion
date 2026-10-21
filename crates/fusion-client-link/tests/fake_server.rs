//! The desktop link against a server that does odd things.

use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::{Duration, Instant};

use fusion_client_link::{Link, LinkEvent};
use fusion_protocol::encode;
use fusion_protocol::v1::{Ack, ServerMessage, server_message};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

/// Runs `body` with a listener on a free local port, on a thread of its own.
fn fake_server<F>(
    body: impl FnOnce(tokio::net::TcpListener) -> F + Send + 'static,
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
            body(tokio::net::TcpListener::from_std(listener).expect("a listener")).await;
        });
    });
    (address, server)
}

fn ack(request: u64) -> ServerMessage {
    ServerMessage {
        body: Some(server_message::Body::Ack(Ack {
            request,
            result: None,
        })),
    }
}

/// A server that sends one connection a bad frame, a text frame, an ack,
/// and then closes it.
async fn odd_frames(listener: tokio::net::TcpListener) {
    let (tcp, _) = listener.accept().await.expect("a client");
    let mut ws = tokio_tungstenite::accept_async(tcp)
        .await
        .expect("a WebSocket");
    for frame in [
        Message::Binary(vec![0xff].into()),
        Message::Text("hello".into()),
        Message::Binary(encode(&ack(7)).into()),
    ] {
        ws.send(frame).await.expect("sent");
    }
    ws.close(None).await.expect("closed");
}

#[test]
fn odd_frames_are_skipped_and_a_close_is_a_disconnect() {
    let (address, server) = fake_server(odd_frames);
    let mut link = Link::connect(&format!("ws://{address}"), || {}).expect("starts");
    let start = Instant::now();
    let mut seen = Vec::new();
    while !seen.iter().any(|e| matches!(e, LinkEvent::Disconnected(_))) {
        assert!(start.elapsed() < Duration::from_secs(15), "saw {seen:?}");
        seen.extend(link.poll());
        thread::sleep(Duration::from_millis(10));
    }
    server.join().expect("the server ended");
    assert_eq!(
        seen,
        [
            LinkEvent::Connected,
            LinkEvent::Message(ack(7)),
            LinkEvent::Disconnected("closed by the server".to_owned()),
        ]
    );
}

#[test]
fn a_link_dropped_before_the_handshake_ends_its_thread() {
    let (dropped, wait_for_drop) = std_mpsc::channel::<()>();
    let (address, server) = fake_server(move |listener| async move {
        let (tcp, _) = listener.accept().await.expect("a client");
        // Finish the handshake only once nobody waits for the link.
        let _gone = wait_for_drop.recv();
        let mut ws = tokio_tungstenite::accept_async(tcp)
            .await
            .expect("a WebSocket");
        let next = ws.next().await;
        assert!(
            !matches!(next, Some(Ok(Message::Binary(_) | Message::Text(_)))),
            "the link went away, it did not talk: {next:?}"
        );
    });
    let link = Link::connect(&format!("ws://{address}"), || {}).expect("starts");
    drop(link);
    drop(dropped);
    server.join().expect("the link closed its side");
}
