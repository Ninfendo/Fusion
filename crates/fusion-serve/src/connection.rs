//! Accepting connections, and one connection's reader and writer.

use fusion_protocol::decode;
use fusion_protocol::v1::ClientMessage;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::Incoming;

/// Accepts connections until the core stops.
pub(crate) async fn accept(listener: TcpListener, core: mpsc::Sender<Incoming>, queue: usize) {
    for id in 1u64.. {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        // Small messages: send each at once. A socket that refuses is
        // still usable.
        let _nodelay = tcp.set_nodelay(true);
        tokio::spawn(run(id, tcp, core.clone(), queue));
    }
}

/// One client: frames in go to the core, frames from the core go out.
async fn run(id: u64, tcp: TcpStream, core: mpsc::Sender<Incoming>, queue: usize) {
    let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else {
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(queue);
    if core.send(Incoming::Opened(id, tx)).await.is_err() {
        return;
    }
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if sink.send(Message::Binary(frame.into())).await.is_err() {
                return;
            }
        }
    });
    while let Some(Ok(message)) = stream.next().await {
        let item = match message {
            Message::Binary(bytes) => match decode::<ClientMessage>(&bytes) {
                Ok(message) => Incoming::Message(id, message),
                Err(_) => Incoming::NotAMessage(id),
            },
            Message::Text(_) => Incoming::NotAMessage(id),
            // Pings, pongs and the close frame: the stream ends after a close.
            _ => continue,
        };
        if core.send(item).await.is_err() {
            break;
        }
    }
    // The core may have stopped; then nobody needs to know.
    let _told = core.send(Incoming::Closed(id)).await;
    writer.abort();
}
