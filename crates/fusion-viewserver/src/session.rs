//! One client's session: its viewports, and when each is due.

use std::collections::BTreeMap;

use fusion_protocol::v1::{
    self, Ack, ClientMessage, ServerMessage, client_message, server_message,
};

use crate::table::Table;
use crate::viewport::Viewport;

/// Every table the server keeps, by name.
#[derive(Debug, Default)]
pub struct Tables {
    tables: BTreeMap<String, Table>,
}

impl Tables {
    /// Adds a table, replacing one of the same name.
    pub fn insert(&mut self, table: Table) {
        self.tables.insert(table.name().to_owned(), table);
    }

    /// A table by name.
    pub fn get(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }

    /// A table by name, to change.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.get_mut(name)
    }

    /// The number of tables.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

/// A layout request: the session leaves it to whoever stores layouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutRequest {
    /// Store a layout under a name.
    Save {
        /// The client's request number.
        request: u64,
        /// The name.
        name: String,
        /// The layout as JSON.
        json: String,
    },
    /// Read the layout stored under a name.
    Load {
        /// The client's request number.
        request: u64,
        /// The name.
        name: String,
    },
}

struct Open {
    viewport: Viewport,
    /// When it may send next, in milliseconds on the server's clock.
    due: u64,
}

/// One client's viewports.
#[derive(Default)]
pub struct Session {
    open: BTreeMap<u32, Open>,
}

/// An `Error` message.
pub fn error(request: u64, viewport: u32, message: impl Into<String>) -> ServerMessage {
    ServerMessage {
        body: Some(server_message::Body::Error(v1::Error {
            request,
            viewport,
            message: message.into(),
        })),
    }
}

/// An `Ack` message with no result.
pub fn ack(request: u64) -> ServerMessage {
    ServerMessage {
        body: Some(server_message::Body::Ack(Ack {
            request,
            result: None,
        })),
    }
}

impl Session {
    /// The number of open viewports.
    pub fn viewports(&self) -> usize {
        self.open.len()
    }

    /// Takes a message from the client. Returns the replies to send now,
    /// and a layout request if it was one. A new or changed viewport sends
    /// its snapshot at the next [`Session::poll`].
    pub fn handle(
        &mut self,
        message: ClientMessage,
        tables: &Tables,
    ) -> (Vec<ServerMessage>, Option<LayoutRequest>) {
        use client_message::Body;
        let request = message.request;
        let reply = match message.body {
            Some(Body::CreateViewport(c)) => self.open(request, c.viewport, c.spec, tables, false),
            Some(Body::ChangeViewport(c)) => self.open(request, c.viewport, c.spec, tables, true),
            Some(Body::RemoveViewport(r)) => match self.open.remove(&r.viewport) {
                Some(_) => ack(request),
                None => error(request, r.viewport, "there is no such viewport"),
            },
            Some(Body::SaveLayout(s)) => {
                let save = LayoutRequest::Save {
                    request,
                    name: s.name,
                    json: s.json,
                };
                return (Vec::new(), Some(save));
            }
            Some(Body::LoadLayout(l)) => {
                let load = LayoutRequest::Load {
                    request,
                    name: l.name,
                };
                return (Vec::new(), Some(load));
            }
            None => error(request, 0, "an empty message"),
        };
        (vec![reply], None)
    }

    fn open(
        &mut self,
        request: u64,
        id: u32,
        spec: Option<v1::ViewportSpec>,
        tables: &Tables,
        change: bool,
    ) -> ServerMessage {
        if change != self.open.contains_key(&id) {
            let why = if change {
                "there is no such viewport"
            } else {
                "that viewport is open already"
            };
            return error(request, id, why);
        }
        let Some(spec) = spec else {
            return error(request, id, "a viewport needs a spec");
        };
        let Some(table) = tables.get(&spec.table) else {
            return error(request, id, format!("there is no table {}", spec.table));
        };
        match Viewport::open(id, &spec, table) {
            Ok(viewport) => {
                self.open.insert(id, Open { viewport, due: 0 });
                ack(request)
            }
            Err(err) => error(request, id, err.to_string()),
        }
    }

    /// The messages of every viewport that is due at `now` (milliseconds).
    /// A viewport that sent waits `1000 / rate` ms before it may send again.
    pub fn poll(&mut self, tables: &Tables, now: u64) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        for open in self.open.values_mut() {
            if now < open.due {
                continue;
            }
            let Some(table) = tables.get(open.viewport.table()) else {
                continue;
            };
            if let Some(message) = open.viewport.poll(table) {
                out.push(message);
                open.due = now + 1000 / u64::from(open.viewport.rate_hz().max(1));
            }
        }
        out
    }

    /// Every viewport sends a snapshot next: for a client that fell behind.
    pub fn reset(&mut self) {
        for open in self.open.values_mut() {
            open.viewport.reset();
            open.due = 0;
        }
    }
}

#[cfg(test)]
mod tests;
