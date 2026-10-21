//! The link on desktop: a thread with its own WebSocket.

use std::sync::mpsc as std_mpsc;
use std::thread;

use fusion_protocol::v1::{ClientMessage, ServerMessage};
use fusion_protocol::{decode, encode};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::{LinkEvent, Wake, retry_after};

/// Runs after each event. Boxed, so one copy of the link's code serves
/// every caller.
type BoxedWake = Box<dyn Fn() + Send>;

/// The link on desktop.
pub struct NativeLink {
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    incoming: std_mpsc::Receiver<LinkEvent>,
}

impl NativeLink {
    /// Starts connecting to `url` (for example `ws://127.0.0.1:7070`) on a
    /// thread of its own. `wake` runs after each event.
    pub fn connect(url: &str, wake: impl Wake) -> std::io::Result<Self> {
        Self::start(url.to_owned(), Box::new(wake))
    }

    fn start(url: String, wake: BoxedWake) -> std::io::Result<Self> {
        let (outgoing, from_ui) = mpsc::unbounded_channel();
        let (to_ui, incoming) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        thread::Builder::new()
            .name("fusion-link".to_owned())
            .spawn(move || runtime.block_on(run(url, from_ui, to_ui, wake)))?;
        Ok(Self { outgoing, incoming })
    }

    /// Sends a message; dropped while not connected.
    pub fn send(&mut self, message: &ClientMessage) {
        // The thread lives as long as the link.
        let _sent = self.outgoing.send(encode(message));
    }

    /// The events since the last call.
    pub fn poll(&mut self) -> Vec<LinkEvent> {
        self.incoming.try_iter().collect()
    }
}

/// Connects, reads and writes, and connects again, until the link is
/// dropped.
async fn run(
    url: String,
    mut from_ui: mpsc::UnboundedReceiver<Vec<u8>>,
    to_ui: std_mpsc::Sender<LinkEvent>,
    wake: BoxedWake,
) {
    // TLS for wss:// addresses; installing twice is fine.
    let _installed = rustls::crypto::ring::default_provider().install_default();
    let mut failures = 0;
    loop {
        let ended = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((ws, _)) => {
                failures = 0;
                if to_ui.send(LinkEvent::Connected).is_err() {
                    return;
                }
                wake();
                // Messages sent while away are stale: the client opens its
                // viewports again on Connected.
                while from_ui.try_recv().is_ok() {}
                session(ws, &mut from_ui, &to_ui, &wake).await
            }
            Err(err) => Ended::Lost(err.to_string()),
        };
        let why = match ended {
            Ended::Stopped => return,
            Ended::Lost(why) => why,
        };
        if to_ui.send(LinkEvent::Disconnected(why)).is_err() {
            return;
        }
        wake();
        tokio::time::sleep(retry_after(failures)).await;
        failures = failures.saturating_add(1);
    }
}

enum Ended {
    /// The link was dropped.
    Stopped,
    /// The connection ended.
    Lost(String),
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn session(
    ws: Socket,
    from_ui: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    to_ui: &std_mpsc::Sender<LinkEvent>,
    wake: &dyn Fn(),
) -> Ended {
    let (mut sink, mut stream) = ws.split();
    loop {
        tokio::select! {
            out = from_ui.recv() => match out {
                Some(frame) => {
                    if let Err(err) = sink.send(Message::Binary(frame.into())).await {
                        return Ended::Lost(err.to_string());
                    }
                }
                None => return Ended::Stopped,
            },
            frame = stream.next() => {
                let event = match frame {
                    Some(Ok(Message::Binary(bytes))) => match decode::<ServerMessage>(&bytes) {
                        Ok(message) => LinkEvent::Message(message),
                        // Not from a Fusion server: skip it.
                        Err(_) => continue,
                    },
                    Some(Ok(Message::Close(_))) | None => return Ended::Lost("closed by the server".to_owned()),
                    Some(Ok(_)) => continue,
                    Some(Err(err)) => return Ended::Lost(err.to_string()),
                };
                if to_ui.send(event).is_err() {
                    return Ended::Stopped;
                }
                wake();
            }
        }
    }
}
