use fusion_protocol::Cell;
use fusion_protocol::v1::{
    ChangeViewport, CreateViewport, LoadLayout, RemoveViewport, SaveLayout, ViewportSpec,
};

use super::*;

fn tables() -> Tables {
    let mut tables = Tables::default();
    let mut prices = Table::new("prices", &["symbol", "bid"]);
    prices.upsert("BTC", vec![Cell::Text("BTC".into()), Cell::Integer(100)]);
    tables.insert(prices);
    tables
}

fn spec(table: &str) -> Option<ViewportSpec> {
    Some(ViewportSpec {
        table: table.to_owned(),
        rate_hz: 10,
        ..ViewportSpec::default()
    })
}

fn create(request: u64, viewport: u32, table: &str) -> ClientMessage {
    ClientMessage {
        request,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport,
            spec: spec(table),
        })),
    }
}

fn change(request: u64, viewport: u32, table: &str) -> ClientMessage {
    ClientMessage {
        request,
        body: Some(client_message::Body::ChangeViewport(ChangeViewport {
            viewport,
            spec: spec(table),
        })),
    }
}

fn error_text(message: &ServerMessage) -> &str {
    match &message.body {
        Some(server_message::Body::Error(e)) => &e.message,
        other => panic!("not an error: {other:?}"),
    }
}

fn is_ack(message: &ServerMessage, request: u64) -> bool {
    matches!(&message.body, Some(server_message::Body::Ack(a)) if a.request == request)
}

#[test]
fn a_new_viewport_is_acked_and_sends_its_snapshot_at_the_next_poll() {
    let tables = tables();
    let mut session = Session::default();
    let (replies, layout) = session.handle(create(7, 1, "prices"), &tables);
    assert!(layout.is_none());
    assert!(is_ack(&replies[0], 7));
    assert_eq!(session.viewports(), 1);
    let sent = session.poll(&tables, 0);
    assert!(matches!(
        sent[0].body,
        Some(server_message::Body::Snapshot(_))
    ));
}

#[test]
fn a_viewport_waits_its_rate_before_it_sends_again() {
    let mut tables = tables();
    let mut session = Session::default();
    session.handle(create(1, 1, "prices"), &tables);
    session.poll(&tables, 1000);
    fn set(tables: &mut Tables, value: i64) {
        tables
            .get_mut("prices")
            .expect("prices")
            .set("BTC", 1, Cell::Integer(value));
    }
    set(&mut tables, 101);
    // 10 a second: not before 1100.
    assert!(session.poll(&tables, 1050).is_empty());
    assert_eq!(session.poll(&tables, 1100).len(), 1);
    // Nothing changed: nothing sent, and the next change goes at once.
    assert!(session.poll(&tables, 1300).is_empty());
    set(&mut tables, 102);
    assert_eq!(session.poll(&tables, 1301).len(), 1);
}

#[test]
fn changing_and_removing_a_viewport() {
    let mut tables = tables();
    tables.insert(Table::new("other", &["x"]));
    let mut session = Session::default();
    session.handle(create(1, 1, "prices"), &tables);
    let (replies, _) = session.handle(change(2, 1, "other"), &tables);
    assert!(is_ack(&replies[0], 2));
    let remove = ClientMessage {
        request: 3,
        body: Some(client_message::Body::RemoveViewport(RemoveViewport {
            viewport: 1,
        })),
    };
    let (replies, _) = session.handle(remove.clone(), &tables);
    assert!(is_ack(&replies[0], 3));
    assert_eq!(session.viewports(), 0);
    let (replies, _) = session.handle(remove, &tables);
    assert_eq!(error_text(&replies[0]), "there is no such viewport");
}

#[test]
fn bad_requests_get_errors() {
    let tables = tables();
    let mut session = Session::default();
    let say = |session: &mut Session, message| {
        let (replies, _) = session.handle(message, &tables);
        error_text(&replies[0]).to_owned()
    };
    assert_eq!(
        say(&mut session, change(1, 9, "prices")),
        "there is no such viewport"
    );
    session.handle(create(2, 1, "prices"), &tables);
    assert_eq!(
        say(&mut session, create(3, 1, "prices")),
        "that viewport is open already"
    );
    assert_eq!(
        say(&mut session, create(4, 2, "nope")),
        "there is no table nope"
    );
    let no_spec = ClientMessage {
        request: 5,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport: 3,
            spec: None,
        })),
    };
    assert_eq!(say(&mut session, no_spec), "a viewport needs a spec");
    let bad_column = ClientMessage {
        request: 6,
        body: Some(client_message::Body::CreateViewport(CreateViewport {
            viewport: 4,
            spec: Some(ViewportSpec {
                table: "prices".into(),
                columns: vec!["ask".into()],
                ..ViewportSpec::default()
            }),
        })),
    };
    assert_eq!(say(&mut session, bad_column), "there is no column ask");
    let empty = ClientMessage {
        request: 7,
        body: None,
    };
    assert_eq!(say(&mut session, empty), "an empty message");
    assert_eq!(session.viewports(), 1, "only the good one opened");
}

#[test]
fn layout_requests_are_passed_on() {
    let tables = tables();
    let mut session = Session::default();
    let save = ClientMessage {
        request: 1,
        body: Some(client_message::Body::SaveLayout(SaveLayout {
            name: "main".into(),
            json: "{}".into(),
        })),
    };
    assert_eq!(
        session.handle(save, &tables),
        (
            Vec::new(),
            Some(LayoutRequest::Save {
                request: 1,
                name: "main".into(),
                json: "{}".into()
            })
        )
    );
    let load = ClientMessage {
        request: 2,
        body: Some(client_message::Body::LoadLayout(LoadLayout {
            name: "main".into(),
        })),
    };
    assert_eq!(
        session.handle(load, &tables).1,
        Some(LayoutRequest::Load {
            request: 2,
            name: "main".into()
        })
    );
}

#[test]
fn a_reset_sends_every_snapshot_again_at_once() {
    let tables = tables();
    let mut session = Session::default();
    session.handle(create(1, 1, "prices"), &tables);
    session.poll(&tables, 0);
    assert!(session.poll(&tables, 5000).is_empty());
    session.reset();
    let again = session.poll(&tables, 5001);
    assert!(matches!(
        again[0].body,
        Some(server_message::Body::Snapshot(_))
    ));
}

#[test]
fn a_viewport_whose_table_is_gone_sends_nothing() {
    let mut tables = tables();
    let mut session = Session::default();
    session.handle(create(1, 1, "prices"), &tables);
    tables = Tables::default();
    assert!(tables.is_empty());
    assert!(session.poll(&tables, 0).is_empty());
}

#[test]
fn tables_are_kept_by_name() {
    let mut tables = tables();
    assert_eq!(tables.len(), 1);
    assert!(!tables.is_empty());
    tables.insert(Table::new("prices", &["only"]));
    assert_eq!(tables.len(), 1, "replaced");
    assert_eq!(tables.get("prices").map(|t| t.columns().len()), Some(1));
    assert!(tables.get_mut("missing").is_none());
}
