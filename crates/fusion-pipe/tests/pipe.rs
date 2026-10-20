//! The window bus and the hub server over a real local socket.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use fusion_hub::{Bus, ConnId, FromHub, ToHub};
use fusion_layout::{PanelId, WindowId};
use fusion_pipe::{HubServer, PipeBus, PipeEvent};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};

const WAIT: Duration = Duration::from_secs(10);

/// A socket name no other test or test run uses.
fn name(test: &str) -> String {
    format!("fusion-test-{}-{test}.sock", std::process::id())
}

/// Polls the bus until it has messages.
fn messages(bus: &mut PipeBus) -> Vec<FromHub> {
    let start = Instant::now();
    loop {
        let messages = bus.poll();
        if !messages.is_empty() || start.elapsed() > WAIT {
            return messages;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn hello_from(server: &mut HubServer, window: WindowId) -> ConnId {
    match server.next(WAIT) {
        Some(PipeEvent::Message(conn, ToHub::Hello { window: w })) if w == window => conn,
        other => panic!("expected a hello, got {other:?}"),
    }
}

#[test]
fn a_window_and_the_hub_talk() {
    let name = name("talk");
    let mut server = HubServer::listen(&name).expect("listens");
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    let mut bus = PipeBus::connect(&name, WindowId(2), move || {
        woken.fetch_add(1, Ordering::SeqCst);
    })
    .expect("connects");
    assert!(bus.can_open_windows());
    let conn = hello_from(&mut server, WindowId(2));
    assert!(server.is_open(conn));

    let notice = FromHub::Notice {
        text: "hi".to_owned(),
    };
    server.send(&[conn, ConnId(99)], &notice);
    assert_eq!(messages(&mut bus), [notice]);
    assert!(wakes.load(Ordering::SeqCst) >= 1, "the window was woken");

    bus.send(ToHub::Focused { panel: PanelId(1) });
    assert_eq!(
        server.next(WAIT),
        Some(PipeEvent::Message(
            conn,
            ToHub::Focused { panel: PanelId(1) }
        ))
    );
}

/// A window process's connection without the bus, which keeps its pipe
/// open until the process ends.
fn raw_window(name: &str, window: u32) -> Stream {
    let mut raw =
        Stream::connect(name.to_ns_name::<GenericNamespaced>().expect("name")).expect("connects");
    let hello = format!(
        "{{\"hello\":{{\"window\":{window}}}}}
"
    );
    raw.write_all(hello.as_bytes()).expect("writes");
    raw
}

#[test]
fn a_closed_connection_is_reported_gone() {
    let name = name("closed");
    let mut server = HubServer::listen(&name).expect("listens");
    let raw = raw_window(&name, 2);
    let conn = hello_from(&mut server, WindowId(2));
    drop(raw);
    assert_eq!(server.next(WAIT), Some(PipeEvent::Gone(conn)));
    assert!(!server.is_open(conn));
}

#[test]
fn nothing_happening_ends_the_wait() {
    let mut server = HubServer::listen(&name("quiet")).expect("listens");
    assert_eq!(server.next(Duration::from_millis(20)), None);
}

#[test]
fn a_line_that_is_not_a_message_is_reported() {
    let name = name("bad");
    let mut server = HubServer::listen(&name).expect("listens");
    let mut raw = Stream::connect(
        name.as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("name"),
    )
    .expect("connects");
    raw.write_all(b"nope\n").expect("writes");
    match server.next(WAIT) {
        Some(PipeEvent::BadLine(_, error)) => assert!(!error.is_empty()),
        other => panic!("expected a bad line, got {other:?}"),
    }
}

#[test]
fn a_window_that_went_away_is_dropped_when_written_to() {
    let name = name("dead");
    let mut server = HubServer::listen(&name).expect("listens");
    let raw = raw_window(&name, 3);
    let conn = hello_from(&mut server, WindowId(3));
    drop(raw);
    let start = Instant::now();
    while server.is_open(conn) && start.elapsed() < WAIT {
        server.send(&[conn], &FromHub::Quit);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!server.is_open(conn));
}

#[test]
fn a_window_quits_when_the_hub_goes_away() {
    let name = name("gone");
    let listener = ListenerOptions::new()
        .name(
            name.as_str()
                .to_ns_name::<GenericNamespaced>()
                .expect("name"),
        )
        .create_sync()
        .expect("listens");
    let mut bus = PipeBus::connect(&name, WindowId(2), || {}).expect("connects");
    let hub_end = listener.accept().expect("accepts");
    drop(hub_end);
    assert_eq!(messages(&mut bus), [FromHub::Quit]);
}

#[test]
fn a_message_from_the_hub_that_does_not_read_becomes_a_notice() {
    let name = name("garbled");
    let listener = ListenerOptions::new()
        .name(
            name.as_str()
                .to_ns_name::<GenericNamespaced>()
                .expect("name"),
        )
        .create_sync()
        .expect("listens");
    let mut bus = PipeBus::connect(&name, WindowId(2), || {}).expect("connects");
    let mut hub_end = listener.accept().expect("accepts");
    hub_end.write_all(b"nope\n").expect("writes");
    match messages(&mut bus).as_slice() {
        [FromHub::Notice { text }] => {
            assert!(text.starts_with("A message from the hub could not be read"));
        }
        other => panic!("expected a notice, got {other:?}"),
    }
}

#[test]
fn a_stopped_server_frees_its_name() {
    let name = name("again");
    let server = HubServer::listen(&name).expect("listens");
    assert!(HubServer::listen(&name).is_err(), "the name is taken");
    drop(server);
    assert!(HubServer::listen(&name).is_ok(), "the name is free again");
}

#[test]
fn each_window_has_its_own_connection() {
    let name = name("two");
    let mut server = HubServer::listen(&name).expect("listens");
    let mut first = PipeBus::connect(&name, WindowId(1), || {}).expect("connects");
    let first_conn = hello_from(&mut server, WindowId(1));
    let mut second = PipeBus::connect(&name, WindowId(2), || {}).expect("connects");
    let second_conn = hello_from(&mut server, WindowId(2));
    assert_ne!(first_conn, second_conn);

    server.send(&[second_conn], &FromHub::Quit);
    assert_eq!(messages(&mut second), [FromHub::Quit]);
    assert!(first.poll().is_empty(), "the first window got nothing");
}
