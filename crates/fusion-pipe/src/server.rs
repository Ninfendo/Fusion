//! The launcher's end of the pipes.

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fusion_hub::{ConnId, FromHub, ToHub, encode};
use interprocess::local_socket::{
    Listener, ListenerOptions, RecvHalf, SendHalf, Stream, prelude::*,
};

use crate::lines::read_lines;
use crate::socket_name;

/// What happened on the hub's socket.
#[derive(Debug, Clone, PartialEq)]
pub enum PipeEvent {
    /// A window process sent a message.
    Message(ConnId, ToHub),
    /// A window process sent a line that is not a message.
    BadLine(ConnId, String),
    /// A window process's connection closed.
    Gone(ConnId),
}

enum Incoming {
    Connected(ConnId, SendHalf),
    Event(PipeEvent),
}

/// The launcher's end of the pipes. Each window process that connects gets
/// a reader thread; [`HubServer::next`] hands out what they read.
pub struct HubServer {
    name: String,
    incoming: Receiver<Incoming>,
    writers: BTreeMap<ConnId, SendHalf>,
    stop: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
}

impl HubServer {
    /// Listens on the socket `name`.
    pub fn listen(name: &str) -> io::Result<Self> {
        let listener = ListenerOptions::new()
            .name(socket_name(name)?)
            .create_sync()?;
        let (sender, incoming) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let acceptor = thread::Builder::new()
            .name("fusion-hub-accept".to_owned())
            .spawn(move || accept_all(&listener, &sender, &stopped))?;
        Ok(Self {
            name: name.to_owned(),
            incoming,
            writers: BTreeMap::new(),
            stop,
            acceptor: Some(acceptor),
        })
    }

    /// The next event, or `None` if nothing happens within `timeout`.
    pub fn next(&mut self, timeout: Duration) -> Option<PipeEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.incoming.recv_timeout(left).ok()? {
                Incoming::Connected(conn, writer) => {
                    self.writers.insert(conn, writer);
                }
                Incoming::Event(event) => {
                    if let PipeEvent::Gone(conn) = &event {
                        self.writers.remove(conn);
                    }
                    return Some(event);
                }
            }
        }
    }

    /// True while a connection is open as far as the server knows.
    pub fn is_open(&self, conn: ConnId) -> bool {
        self.writers.contains_key(&conn)
    }

    /// Writes one message to each connection in `to`. The message is
    /// encoded once. A connection that cannot take it is dropped; its reader
    /// then reports it gone.
    pub fn send(&mut self, to: &[ConnId], message: &FromHub) {
        let Ok(line) = encode(message) else {
            return;
        };
        for conn in to {
            let failed = self
                .writers
                .get_mut(conn)
                .is_some_and(|writer| writer.write_all(line.as_bytes()).is_err());
            if failed {
                self.writers.remove(conn);
            }
        }
    }
}

impl Drop for HubServer {
    /// Stops accepting and frees the socket name. The acceptor thread waits
    /// in `accept`, so one last connection wakes it.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The connection stays open until the acceptor has seen it: a
        // connection that closes first may never reach accept.
        let wake = socket_name(&self.name).and_then(Stream::connect);
        let woken = wake.is_ok();
        if let Some(acceptor) = self.acceptor.take().filter(|_| woken) {
            // A panic in the acceptor has nothing left to clean up.
            let _ended = acceptor.join();
        }
    }
}

fn accept_all(listener: &Listener, sender: &Sender<Incoming>, stop: &AtomicBool) {
    let mut next = 0;
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        // A failed accept affects only the process that tried to connect.
        let Ok(stream) = stream else {
            continue;
        };
        let conn = ConnId(next);
        next += 1;
        let (reader, writer) = stream.split();
        if sender.send(Incoming::Connected(conn, writer)).is_err() {
            return;
        }
        let reader_sender = sender.clone();
        let spawned = thread::Builder::new()
            .name(format!("fusion-hub-conn-{}", conn.0))
            .spawn(move || read_conn(conn, reader, &reader_sender));
        if spawned.is_err() {
            // The reader half is gone with the closure; dropping the writer
            // as well closes the pipe, and the window process sees that.
            let _server_gone = sender.send(Incoming::Event(PipeEvent::Gone(conn)));
        }
    }
}

fn read_conn(conn: ConnId, reader: RecvHalf, sender: &Sender<Incoming>) {
    read_lines(reader, |message| {
        let event = match message {
            Ok(message) => PipeEvent::Message(conn, message),
            Err(err) => PipeEvent::BadLine(conn, err.to_string()),
        };
        sender.send(Incoming::Event(event)).is_ok()
    });
    let _server_gone = sender.send(Incoming::Event(PipeEvent::Gone(conn)));
}
