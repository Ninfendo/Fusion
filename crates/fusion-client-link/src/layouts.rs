//! One layout, kept on the view server.

use fusion_protocol::v1::{
    ClientMessage, LoadLayout, SaveLayout, ServerMessage, ack, client_message, server_message,
};

use crate::{Link, LinkEvent, Wake};

/// What came back from the view server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEvent {
    /// The answer to the load: the layout's JSON, `None` when the server
    /// has none by that name yet, or why the load failed.
    Loaded(Result<Option<String>, String>),
    /// A save failed, and why.
    SaveFailed(String),
}

/// Where the load is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    /// To ask on the next connection.
    Wanted,
    /// Asked, with this request number.
    Asked(u64),
    /// Answered.
    Done,
}

/// One layout, by name, kept on the view server. It is loaded once, on
/// the first connection. Saves wait until the load is answered, so the
/// load always shows what was there before. Each save replaces the one
/// before; the newest is sent again after a lost connection until the
/// server confirms it.
pub struct RemoteLayouts {
    link: Link,
    name: String,
    connected: bool,
    next_request: u64,
    load: Load,
    /// The newest layout not sent yet.
    unsent: Option<String>,
    /// The layout sent and not confirmed yet, with its request number.
    in_flight: Option<(u64, String)>,
}

impl RemoteLayouts {
    /// Starts connecting to the view server at `url`, for the layout called
    /// `name`. `wake` runs when something comes in.
    pub fn connect(url: &str, name: &str, wake: impl Wake) -> std::io::Result<Self> {
        Ok(Self {
            link: Link::connect(url, wake)?,
            name: name.to_owned(),
            connected: false,
            next_request: 1,
            load: Load::Wanted,
            unsent: None,
            in_flight: None,
        })
    }

    /// Stores `json` as the layout: at once when connected and loaded,
    /// else as soon as that is so.
    pub fn save(&mut self, json: String) {
        self.unsent = Some(json);
        self.send_unsent();
    }

    /// True while a saved layout is not confirmed by the server.
    pub fn pending(&self) -> bool {
        self.unsent.is_some() || self.in_flight.is_some()
    }

    /// Takes in what the link brought.
    pub fn poll(&mut self) -> Vec<RemoteEvent> {
        let mut events = Vec::new();
        for event in self.link.poll() {
            match event {
                LinkEvent::Connected => self.on_connected(),
                LinkEvent::Disconnected(_) => self.on_disconnected(),
                LinkEvent::Message(message) => events.extend(self.on_message(message)),
            }
        }
        events
    }

    fn request(&mut self, body: client_message::Body) -> u64 {
        let request = self.next_request;
        self.next_request += 1;
        self.link.send(&ClientMessage {
            request,
            body: Some(body),
        });
        request
    }

    fn on_connected(&mut self) {
        self.connected = true;
        if self.load == Load::Wanted {
            let name = self.name.clone();
            let request = self.request(client_message::Body::LoadLayout(LoadLayout { name }));
            self.load = Load::Asked(request);
        }
        self.send_unsent();
    }

    /// Whatever was not answered is asked again on the next connection.
    fn on_disconnected(&mut self) {
        self.connected = false;
        if let Load::Asked(_) = self.load {
            self.load = Load::Wanted;
        }
        if let Some((_, json)) = self.in_flight.take() {
            self.unsent.get_or_insert(json);
        }
    }

    fn send_unsent(&mut self) {
        if !self.connected || self.load != Load::Done {
            return;
        }
        if let Some(json) = self.unsent.take() {
            let save = SaveLayout {
                name: self.name.clone(),
                json: json.clone(),
            };
            let request = self.request(client_message::Body::SaveLayout(save));
            self.in_flight = Some((request, json));
        }
    }

    fn on_message(&mut self, message: ServerMessage) -> Option<RemoteEvent> {
        let (request, answer) = match message.body? {
            server_message::Body::Ack(ack) => (
                ack.request,
                Ok(ack.result.map(|ack::Result::LayoutJson(json)| json)),
            ),
            server_message::Body::Error(error) => (error.request, Err(error.message)),
            _ => return None,
        };
        if self.load == Load::Asked(request) {
            self.load = Load::Done;
            self.send_unsent();
            return Some(RemoteEvent::Loaded(answer));
        }
        if self.in_flight.as_ref().is_some_and(|(r, _)| *r == request) {
            self.in_flight = None;
            return answer.err().map(RemoteEvent::SaveFailed);
        }
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RemoteLayouts {
    /// Waits up to `wait` for the answer to the load. `None` if it did not
    /// come in time. Other events that come meanwhile are dropped.
    pub fn wait_loaded(
        &mut self,
        wait: std::time::Duration,
    ) -> Option<Result<Option<String>, String>> {
        let end = std::time::Instant::now() + wait;
        loop {
            let loaded = self.poll().into_iter().find_map(|event| match event {
                RemoteEvent::Loaded(answer) => Some(answer),
                RemoteEvent::SaveFailed(_) => None,
            });
            if loaded.is_some() || std::time::Instant::now() >= end {
                return loaded;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Waits up to `wait` until the server confirmed the last save. Returns
    /// the failures that came in meanwhile.
    pub fn settle(&mut self, wait: std::time::Duration) -> Vec<RemoteEvent> {
        let end = std::time::Instant::now() + wait;
        let mut events = self.poll();
        while self.pending() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
            events.extend(self.poll());
        }
        events
    }
}
