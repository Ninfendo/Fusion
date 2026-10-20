//! The view server over WebSocket.
//!
//! One core task owns every table and every client session: it applies
//! market events to the tables, handles client requests, and every
//! [`TICK`] sends each session what is due. Each connection has a reader
//! and a writer task; the core never waits for a client. A client whose
//! queue of [`ServeConfig::queue`] frames is full gets nothing more until
//! it has drained half, and then a fresh snapshot of each viewport instead
//! of the backlog. Layout requests run on their own tasks, so a slow
//! database never holds up prices.

mod connection;
mod layouts;
mod outbox;

use std::collections::BTreeMap;
use std::time::Duration;

use fusion_domain::MarketEvent;
use fusion_protocol::encode;
use fusion_protocol::v1::{self, ClientMessage, ServerMessage, ack, server_message};
use fusion_viewserver::{LayoutRequest, MarketTables, Session, Tables, error};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub use layouts::{LayoutStore, MemoryLayouts};

use outbox::{Delivery, Outbox};

/// How often the core sends what is due.
pub const TICK: Duration = Duration::from_millis(10);

/// How the server behaves.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Frames queued for one client at most.
    pub queue: usize,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self { queue: 256 }
    }
}

pub(crate) enum Incoming {
    Opened(u64, mpsc::Sender<Vec<u8>>),
    Message(u64, ClientMessage),
    NotAMessage(u64),
    Closed(u64),
}

struct Client {
    session: Session,
    outbox: Outbox,
    tx: mpsc::Sender<Vec<u8>>,
}

struct Core<S> {
    tables: Tables,
    market: MarketTables,
    clients: BTreeMap<u64, Client>,
    layouts: S,
    started: Instant,
}

/// Serves the market tables of `symbols` to every client that connects to
/// `listener`, until `market` closes. Instrument `n` of the market events
/// is `symbols[n - 1]`.
pub async fn serve<S: LayoutStore>(
    listener: TcpListener,
    symbols: &[String],
    mut market: mpsc::Receiver<MarketEvent>,
    layouts: S,
    config: ServeConfig,
) {
    let mut tables = Tables::default();
    let market_tables = MarketTables::new(symbols, &mut tables);
    let mut core = Core {
        tables,
        market: market_tables,
        clients: BTreeMap::new(),
        layouts,
        started: Instant::now(),
    };
    let (incoming_tx, mut incoming) = mpsc::channel(4096);
    let acceptor = tokio::spawn(connection::accept(listener, incoming_tx, config.queue));
    let mut tick = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            event = market.recv() => match event {
                Some(event) => core.market.apply(&event, &mut core.tables),
                None => break,
            },
            Some(item) = incoming.recv() => core.on_incoming(item),
            _ = tick.tick() => core.poll(),
        }
    }
    acceptor.abort();
}

impl<S: LayoutStore> Core<S> {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn on_incoming(&mut self, item: Incoming) {
        match item {
            Incoming::Opened(id, tx) => {
                let client = Client {
                    session: Session::default(),
                    outbox: Outbox::new(tx.clone()),
                    tx,
                };
                self.clients.insert(id, client);
            }
            Incoming::Message(id, message) => self.on_message(id, message),
            Incoming::NotAMessage(id) => self.reply(id, &[error(0, 0, "not a message")]),
            Incoming::Closed(id) => {
                self.clients.remove(&id);
            }
        }
    }

    fn on_message(&mut self, id: u64, message: ClientMessage) {
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        let (replies, layout) = client.session.handle(message, &self.tables);
        if let Some(request) = layout {
            let store = self.layouts.clone();
            let tx = client.tx.clone();
            tokio::spawn(answer_layout(store, request, tx));
        }
        self.reply(id, &replies);
    }

    fn reply(&mut self, id: u64, messages: &[ServerMessage]) {
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        if client.outbox.deliver(messages, &mut client.session) == Delivery::Gone {
            self.clients.remove(&id);
        }
    }

    fn poll(&mut self) {
        let now = self.now_ms();
        let mut gone = Vec::new();
        for (id, client) in &mut self.clients {
            if client.outbox.waiting() {
                continue;
            }
            let messages = client.session.poll(&self.tables, now);
            if client.outbox.deliver(&messages, &mut client.session) == Delivery::Gone {
                gone.push(*id);
            }
        }
        for id in gone {
            self.clients.remove(&id);
        }
    }
}

/// Answers a layout request from the store, on its own task.
async fn answer_layout<S: LayoutStore>(
    store: S,
    request: LayoutRequest,
    tx: mpsc::Sender<Vec<u8>>,
) {
    let reply = match request {
        LayoutRequest::Save {
            request,
            name,
            json,
        } => match store.save(name, json).await {
            Ok(()) => fusion_viewserver::ack(request),
            Err(why) => error(request, 0, why),
        },
        LayoutRequest::Load { request, name } => match store.load(name.clone()).await {
            Ok(Some(json)) => ServerMessage {
                body: Some(server_message::Body::Ack(v1::Ack {
                    request,
                    result: Some(ack::Result::LayoutJson(json)),
                })),
            },
            Ok(None) => error(request, 0, format!("there is no layout {name}")),
            Err(why) => error(request, 0, why),
        },
    };
    // A client that left needs no answer.
    let _sent = tx.send(encode(&reply)).await;
}
