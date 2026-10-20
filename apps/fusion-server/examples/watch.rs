//! Watches a table on a running server and prints what arrives.
//!
//! `cargo run -p fusion-server --example watch -- ws://127.0.0.1:7070 prices 10`

use std::error::Error;

use fusion_protocol::v1::{
    ClientMessage, CreateViewport, ServerMessage, ViewportSpec, client_message, server_message,
};
use fusion_protocol::{decode, encode};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = args.first().map_or("ws://127.0.0.1:7070", String::as_str);
    let table = args.get(1).map_or("prices", String::as_str);
    let count: usize = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(10);
    let (mut ws, _) = tokio_tungstenite::connect_async(address).await?;
    let open = ClientMessage {
        request: 1,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport: 1,
            spec: Some(ViewportSpec {
                table: table.to_owned(),
                ..ViewportSpec::default()
            }),
        })),
    };
    ws.send(Message::Binary(encode(&open).into())).await?;
    let mut seen = 0;
    while let Some(frame) = ws.next().await {
        let Message::Binary(bytes) = frame? else {
            continue;
        };
        match decode::<ServerMessage>(&bytes)?.body {
            Some(server_message::Body::Snapshot(s)) => {
                println!(
                    "snapshot: {} rows of {}, columns {:?}",
                    s.rows.len(),
                    s.total_rows,
                    s.columns
                );
            }
            Some(server_message::Body::Delta(d)) => {
                let keys: Vec<&str> = d.updated.iter().map(|u| u.key.as_str()).collect();
                println!("delta {}: updated {keys:?}", d.seq);
            }
            other => println!("{other:?}"),
        }
        seen += 1;
        if seen >= count {
            break;
        }
    }
    Ok(())
}
