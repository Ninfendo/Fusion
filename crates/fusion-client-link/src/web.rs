//! The link in the browser: the browser's own WebSocket.

use std::cell::RefCell;
use std::rc::Rc;

use fusion_protocol::v1::{ClientMessage, ServerMessage};
use fusion_protocol::{decode, encode};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

use crate::{LinkEvent, retry_after};

struct Shared {
    url: String,
    socket: Option<WebSocket>,
    events: Vec<LinkEvent>,
    failures: u32,
    wake: Box<dyn Fn()>,
}

/// The link in the browser.
pub struct WebLink {
    shared: Rc<RefCell<Shared>>,
}

impl WebLink {
    /// Starts connecting to `url` (for example `ws://127.0.0.1:7070`).
    /// `wake` runs after each event.
    pub fn connect(url: &str, wake: impl crate::Wake) -> std::io::Result<Self> {
        let shared = Rc::new(RefCell::new(Shared {
            url: url.to_owned(),
            socket: None,
            events: Vec::new(),
            failures: 0,
            wake: Box::new(wake),
        }));
        open(&shared);
        Ok(Self { shared })
    }

    /// Sends a message; dropped while not connected.
    pub fn send(&mut self, message: &ClientMessage) {
        let shared = self.shared.borrow();
        if let Some(socket) = shared
            .socket
            .as_ref()
            .filter(|s| s.ready_state() == WebSocket::OPEN)
        {
            // A socket that refuses is closing; its close event follows.
            let _sent = socket.send_with_u8_array(&encode(message));
        }
    }

    /// The events since the last call.
    pub fn poll(&mut self) -> Vec<LinkEvent> {
        std::mem::take(&mut self.shared.borrow_mut().events)
    }
}

fn push(shared: &Rc<RefCell<Shared>>, event: LinkEvent) {
    shared.borrow_mut().events.push(event);
    (shared.borrow().wake)();
}

/// Opens the WebSocket and wires its events. The closures live as long as
/// the socket: they are handed to JavaScript.
fn open(shared: &Rc<RefCell<Shared>>) {
    let url = shared.borrow().url.clone();
    let Ok(socket) = WebSocket::new(&url) else {
        retry(shared, "the address is not a WebSocket address".to_owned());
        return;
    };
    socket.set_binary_type(BinaryType::Arraybuffer);
    let on_open = {
        let shared = Rc::clone(shared);
        Closure::<dyn FnMut()>::new(move || {
            shared.borrow_mut().failures = 0;
            push(&shared, LinkEvent::Connected);
        })
    };
    let on_message = {
        let shared = Rc::clone(shared);
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
                return;
            };
            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            if let Ok(message) = decode::<ServerMessage>(&bytes) {
                push(&shared, LinkEvent::Message(message));
            }
        })
    };
    let on_close = {
        let shared = Rc::clone(shared);
        Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
            shared.borrow_mut().socket = None;
            retry(&shared, format!("closed ({})", event.code()));
        })
    };
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_open.forget();
    on_message.forget();
    on_close.forget();
    shared.borrow_mut().socket = Some(socket);
}

/// Says the link is down, and opens it again after a wait.
fn retry(shared: &Rc<RefCell<Shared>>, why: String) {
    push(shared, LinkEvent::Disconnected(why));
    let wait = {
        let mut s = shared.borrow_mut();
        let wait = retry_after(s.failures);
        s.failures = s.failures.saturating_add(1);
        wait
    };
    let again = {
        let shared = Rc::clone(shared);
        Closure::once_into_js(move || open(&shared))
    };
    if let Some(window) = web_sys::window() {
        let millis = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
        // A browser that refuses a timer has no page left to show.
        let _timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(again.unchecked_ref(), millis);
    }
}
