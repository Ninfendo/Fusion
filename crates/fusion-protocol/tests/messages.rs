//! Whole messages through the wire format and back.

use fusion_protocol::v1::{
    self, ClientMessage, CreateViewport, Delta, Row, RowUpdate, ServerMessage, Snapshot, Sort,
    ViewportSpec, client_message, server_message,
};
use fusion_protocol::{Cell, decode, encode};

fn value(cell: &Cell) -> v1::Value {
    v1::Value::try_from(cell).expect("fits")
}

#[test]
fn a_viewport_request_survives_the_wire() {
    let message = ClientMessage {
        request: 7,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport: 1,
            spec: Some(ViewportSpec {
                table: "prices".to_owned(),
                columns: vec!["symbol".to_owned(), "bid".to_owned()],
                sort: vec![Sort {
                    column: "volume".to_owned(),
                    descending: true,
                }],
                filter: "symbol.startsWith(\"BTC\")".to_owned(),
                offset: 0,
                limit: 50,
                rate_hz: 0,
            }),
        })),
    };
    let back: ClientMessage = decode(&encode(&message)).expect("decodes");
    assert_eq!(back, message);
}

#[test]
fn snapshots_and_deltas_survive_the_wire() {
    let row = Row {
        key: "BTCUSDT".to_owned(),
        cells: vec![
            value(&Cell::Text("BTCUSDT".to_owned())),
            value(&Cell::Decimal("82731.35".parse().expect("a price"))),
        ],
    };
    let snapshot = ServerMessage {
        body: Some(server_message::Body::Snapshot(Snapshot {
            viewport: 1,
            seq: 1,
            columns: vec!["symbol".to_owned(), "bid".to_owned()],
            total_rows: 12,
            rows: vec![row.clone()],
        })),
    };
    let back: ServerMessage = decode(&encode(&snapshot)).expect("decodes");
    assert_eq!(back, snapshot);

    let delta = ServerMessage {
        body: Some(server_message::Body::Delta(Delta {
            viewport: 1,
            seq: 2,
            total_rows: 12,
            added: vec![row],
            updated: vec![RowUpdate {
                key: "ETHUSDT".to_owned(),
                columns: vec![1],
                values: vec![value(&Cell::Decimal("2448.31".parse().expect("a price")))],
            }],
            removed: vec!["SOLUSDT".to_owned()],
            order: vec!["BTCUSDT".to_owned(), "ETHUSDT".to_owned()],
        })),
    };
    let back: ServerMessage = decode(&encode(&delta)).expect("decodes");
    assert_eq!(back, delta);
}

#[test]
fn bytes_that_are_not_a_message_are_refused() {
    assert!(decode::<ServerMessage>(&[0xff, 0xff, 0xff]).is_err());
}
