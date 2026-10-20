//! One stream connection, on its own task.

use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::Incoming;

/// Connects to `url` and hands every text message to the live loop, until
/// the connection ends; then says why.
pub(super) async fn run(conn: u64, url: String, tx: mpsc::Sender<Incoming>) {
    let reason = match connect_async(url.as_str()).await {
        Err(err) => err.to_string(),
        Ok((stream, _response)) => {
            if tx.send(Incoming::Opened(conn)).await.is_err() {
                return;
            }
            read(conn, stream, &tx).await
        }
    };
    // The loop may have stopped; then nobody needs to know.
    let _told = tx.send(Incoming::Closed(conn, reason)).await;
}

/// Reads until the end. Pings are answered by the library while it reads.
async fn read<S>(conn: u64, mut stream: S, tx: &mpsc::Sender<Incoming>) -> String
where
    S: futures_util::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    while let Some(message) = stream.next().await {
        match message {
            Ok(WsMessage::Text(text)) => {
                if tx.send(Incoming::Text(conn, text)).await.is_err() {
                    return "the feed stopped".to_owned();
                }
            }
            Ok(WsMessage::Close(frame)) => {
                return frame.map_or_else(
                    || "closed by the server".to_owned(),
                    |f| format!("closed by the server: {} {}", u16::from(f.code), f.reason),
                );
            }
            Ok(_) => {}
            Err(err) => return err.to_string(),
        }
    }
    "the stream ended".to_owned()
}
