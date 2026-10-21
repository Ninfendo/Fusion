//! The bus between browser tabs.
//!
//! The main tab runs the hub; the other tabs post to it over a
//! BroadcastChannel. A Web Lock makes sure only one main tab runs the hub; a
//! second copy of the main tab follows it. The rules live in
//! [`fusion_hub::TabHub`] and [`fusion_hub::PendingPopups`]; this file only
//! makes the browser calls.
//!
//! With `?server=`, the hub tab also keeps the layout on the view server.
//! It starts with the local copy, then takes the server's when it comes,
//! unless the user changed the layout meanwhile (see
//! [`fusion_client_core::workspace::choose_layout`]).

use std::cell::RefCell;
use std::rc::Rc;

use eframe::wasm_bindgen::closure::Closure;
use eframe::wasm_bindgen::{JsCast as _, JsValue};
use fusion_client_core::workspace::{LayoutChoice, SERVER_LAYOUT, choose_layout};
use fusion_client_link::{RemoteEvent, RemoteLayouts};
use fusion_hub::{
    Bus, Envelope, FromHub, PendingPopups, Popup, TabHub, TabReply, ToHub, decode, encode,
};
use fusion_layout::{Layout, WindowId};
use js_sys::{Function, Object, Promise, Reflect};
use web_sys::{BroadcastChannel, MessageEvent};

const CHANNEL: &str = "fusion";
const LOCK: &str = "fusion-hub";
/// Where the hub tab keeps the layout.
const LAYOUT_KEY: &str = "fusion.hub.layout";
/// How long after a change the layout is saved, so a drag saves once.
const SAVE_DELAY_MS: i32 = 1000;

enum Role {
    /// Waiting for the lock; messages wait too.
    Deciding(Vec<ToHub>),
    /// This tab runs the hub.
    Hub(TabHub),
    /// Another tab runs the hub.
    Follower,
}

struct Shared {
    window: WindowId,
    role: Role,
    inbox: Vec<FromHub>,
    channel: BroadcastChannel,
    /// Saves the layout; a browser timer calls it. A timer, not the next
    /// frame, because a tab in the background stops drawing.
    saver: Option<Function>,
    save_pending: bool,
    wake: Rc<dyn Fn()>,
    /// The view server's address, from the page's `?server=`.
    server: Option<String>,
    /// The hub tab's copy of the layout on the view server.
    remote: Option<RemoteLayouts>,
    /// The layout changed since the hub started in this tab.
    changed_here: bool,
}

impl Shared {
    fn post(&self, envelope: &Envelope) {
        if let Ok(line) = encode(envelope) {
            // A closed channel means the tab is going away.
            let _posted = self.channel.post_message(&JsValue::from_str(&line));
        }
    }

    fn send(&mut self, message: ToHub) {
        match &mut self.role {
            Role::Deciding(waiting) => waiting.push(message),
            Role::Hub(hub) => {
                let reply = hub.handle(self.window, message);
                self.take(reply);
            }
            Role::Follower => self.post(&Envelope::ToHub {
                from: self.window,
                message,
            }),
        }
    }

    fn take(&mut self, reply: TabReply) {
        self.changed_here |= reply.changed;
        if reply.changed && !self.save_pending {
            let timer = web_sys::window()
                .zip(self.saver.as_ref())
                .map(|(page, saver)| {
                    page.set_timeout_with_callback_and_timeout_and_arguments_0(saver, SAVE_DELAY_MS)
                });
            self.save_pending = matches!(timer, Some(Ok(_)));
        }
        self.inbox.extend(reply.mine);
        for post in &reply.post {
            self.post(post);
        }
        (self.wake)();
    }

    fn receive(&mut self, envelope: Envelope) {
        match (&mut self.role, envelope) {
            (Role::Hub(hub), Envelope::ToHub { from, message }) => {
                let reply = hub.handle(from, message);
                self.take(reply);
            }
            (Role::Hub(_), _) => {}
            (_, Envelope::HubStarted) => {
                let hello = ToHub::Hello {
                    window: self.window,
                };
                self.send(hello);
            }
            (_, envelope) => {
                if let Some(message) = envelope.for_window(self.window) {
                    self.inbox.push(message);
                    (self.wake)();
                }
            }
        }
    }

    /// The lock decided: run the hub here, or follow the tab that does.
    fn decide(&mut self, run_hub: bool) {
        let Role::Deciding(waiting) = std::mem::replace(&mut self.role, Role::Follower) else {
            return;
        };
        if run_hub {
            self.role = Role::Hub(TabHub::new(load_layout()));
            self.post(&Envelope::HubStarted);
            self.remote = self.server.as_deref().and_then(|url| {
                let wake = Rc::clone(&self.wake);
                // A link that cannot start leaves the layout in this browser.
                RemoteLayouts::connect(url, SERVER_LAYOUT, move || wake()).ok()
            });
        }
        for message in waiting {
            self.send(message);
        }
    }

    fn save(&mut self) {
        self.save_pending = false;
        let Role::Hub(hub) = &self.role else {
            return;
        };
        let Ok(json) = hub.layout().to_json() else {
            return;
        };
        if let Some(storage) = local_storage() {
            // A full storage keeps the last saved layout.
            let _saved = storage.set_item(LAYOUT_KEY, &json);
        }
        if let Some(remote) = &mut self.remote {
            remote.save(json);
        }
    }

    /// Takes in what the view server said about the layout.
    fn pump_remote(&mut self) {
        let Some(remote) = &mut self.remote else {
            return;
        };
        for event in remote.poll() {
            match event {
                RemoteEvent::Loaded(answer) => match choose_layout(answer, self.changed_here) {
                    LayoutChoice::UseServer(layout) => self.send(ToHub::ReplaceLayout { layout }),
                    LayoutChoice::SendLocal => self.save(),
                    LayoutChoice::KeepLocal(text) => self.inbox.push(FromHub::Notice { text }),
                },
                RemoteEvent::SaveFailed(why) => self.inbox.push(FromHub::Notice {
                    text: format!("The view server did not keep the layout: {why}"),
                }),
            }
        }
    }

    /// The tab is going away. The hub tab saves and ends every tab; a
    /// popped-out window's panels go back to the main window.
    fn leaving(&mut self) {
        match self.role {
            Role::Hub(_) => {
                self.save();
                self.send(ToHub::CloseWindow {
                    window: WindowId::MAIN,
                });
            }
            // A second copy of the main tab leaves the window to the first.
            _ if self.window == WindowId::MAIN => {}
            _ => self.send(ToHub::CloseWindow {
                window: self.window,
            }),
        }
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn load_layout() -> Layout {
    local_storage()
        .and_then(|s| s.get_item(LAYOUT_KEY).ok()?)
        .and_then(|json| Layout::from_json(&json).ok())
        .unwrap_or_else(fusion_client_core::workspace::default_layout)
}

/// The bus of one browser tab.
pub struct WebBus {
    shared: Rc<RefCell<Shared>>,
    popups: PendingPopups<web_sys::Window>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_leave: Closure<dyn FnMut()>,
    _on_save: Closure<dyn FnMut()>,
}

impl WebBus {
    /// The bus for the tab that shows `window`. `server` is the view
    /// server's address, if the page has one. `wake` asks the tab to draw.
    pub fn new(
        window: WindowId,
        server: Option<String>,
        wake: impl Fn() + 'static,
    ) -> Result<Self, JsValue> {
        let channel = BroadcastChannel::new(CHANNEL)?;
        let role = if window == WindowId::MAIN {
            Role::Deciding(Vec::new())
        } else {
            Role::Follower
        };
        let shared = Rc::new(RefCell::new(Shared {
            window,
            role,
            inbox: Vec::new(),
            channel: channel.clone(),
            saver: None,
            save_pending: false,
            wake: Rc::new(wake),
            server,
            remote: None,
            changed_here: false,
        }));

        let on_save = {
            let shared = Rc::downgrade(&shared);
            Closure::<dyn FnMut()>::new(move || {
                if let Some(shared) = shared.upgrade() {
                    shared.borrow_mut().save();
                }
            })
        };
        shared.borrow_mut().saver = Some(on_save.as_ref().unchecked_ref::<Function>().clone());

        let on_message = {
            let shared = Rc::clone(&shared);
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let Some(text) = event.data().as_string() else {
                    return;
                };
                if let Ok(envelope) = decode::<Envelope>(&text) {
                    shared.borrow_mut().receive(envelope);
                }
            })
        };
        channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_leave = {
            let shared = Rc::clone(&shared);
            Closure::<dyn FnMut()>::new(move || shared.borrow_mut().leaving())
        };
        if let Some(page) = web_sys::window() {
            page.add_event_listener_with_callback("pagehide", on_leave.as_ref().unchecked_ref())?;
        }

        shared.borrow_mut().send(ToHub::Hello { window });
        if window == WindowId::MAIN {
            request_hub_lock(&shared);
        }
        Ok(Self {
            shared,
            popups: PendingPopups::default(),
            _on_message: on_message,
            _on_leave: on_leave,
            _on_save: on_save,
        })
    }

    /// Opens an empty browser window at once, while the click still counts,
    /// and fills it when the hub says where the panel went.
    fn open_popup(&mut self, panel: fusion_layout::PanelId) -> bool {
        let popup = web_sys::window().and_then(|page| {
            page.open_with_url_and_target_and_features(
                "about:blank",
                "_blank",
                "popup,width=960,height=640",
            )
            .ok()
            .flatten()
        });
        match popup {
            Some(popup) => {
                self.popups.add(panel, popup, js_sys::Date::now() / 1000.0);
                true
            }
            None => {
                self.shared.borrow_mut().inbox.push(FromHub::Notice {
                    text: "The browser blocked the new window. Allow pop-ups for this site."
                        .to_owned(),
                });
                false
            }
        }
    }

    fn settle_popups(&mut self, layout: Option<&Layout>) {
        if self.popups.is_empty() {
            return;
        }
        let window = self.shared.borrow().window;
        for popup in self
            .popups
            .settle(layout, window, js_sys::Date::now() / 1000.0)
        {
            match popup {
                Popup::Show(popup, target) => {
                    let location = web_sys::window().map(|page| page.location());
                    let path = location
                        .as_ref()
                        .and_then(|l| l.pathname().ok())
                        .unwrap_or_default();
                    let query = location.and_then(|l| l.search().ok()).unwrap_or_default();
                    let query = fusion_hub::query_for_window(&query, target);
                    // A window that went away cannot be filled; it is gone.
                    let _shown = popup.location().set_href(&format!("{path}{query}"));
                }
                Popup::Close(popup) => {
                    let _closed = popup.close();
                }
            }
        }
    }
}

/// Asks for the hub lock without waiting: the tab that gets it runs the hub
/// and holds it until the tab closes. A browser without Web Locks runs the
/// hub in every main tab.
fn request_hub_lock(shared: &Rc<RefCell<Shared>>) {
    let decide = |shared: &Rc<RefCell<Shared>>, run_hub: bool| shared.borrow_mut().decide(run_hub);
    let locks = web_sys::window()
        .map(|page| page.navigator())
        .and_then(|navigator| Reflect::get(&navigator, &"locks".into()).ok())
        .filter(|locks| !locks.is_undefined());
    let request = locks.as_ref().and_then(|locks| {
        Reflect::get(locks, &"request".into())
            .ok()?
            .dyn_into::<Function>()
            .ok()
    });
    let (Some(locks), Some(request)) = (locks, request) else {
        decide(shared, true);
        return;
    };
    let options = Object::new();
    // Setting a field on a fresh object cannot fail.
    let _set = Reflect::set(&options, &"ifAvailable".into(), &JsValue::TRUE);
    let granted = {
        let shared = Rc::clone(shared);
        Closure::once_into_js(move |lock: JsValue| -> JsValue {
            let run_hub = !lock.is_null();
            decide(&shared, run_hub);
            if run_hub {
                // A promise that never settles holds the lock while the tab lives.
                Promise::new(&mut |_, _| {}).into()
            } else {
                JsValue::UNDEFINED
            }
        })
    };
    if request
        .call3(&locks, &LOCK.into(), &options, &granted)
        .is_err()
    {
        decide(shared, true);
    }
}

impl Bus for WebBus {
    fn send(&mut self, message: ToHub) {
        if let ToHub::PopOut { panel } = message
            && !self.open_popup(panel)
        {
            return;
        }
        self.shared.borrow_mut().send(message);
    }

    fn poll(&mut self) -> Vec<FromHub> {
        self.shared.borrow_mut().pump_remote();
        let messages = std::mem::take(&mut self.shared.borrow_mut().inbox);
        let layout = messages.iter().rev().find_map(|m| match m {
            FromHub::Layout { layout } => Some(layout),
            _ => None,
        });
        self.settle_popups(layout);
        let shared = self.shared.borrow();
        let gone = layout.is_some_and(|l| l.window(shared.window).is_none());
        if shared.window != WindowId::MAIN && (gone || messages.contains(&FromHub::Quit)) {
            // Only a window a script opened can close itself: a popped-out one.
            if let Some(page) = web_sys::window() {
                let _closed = page.close();
            }
        }
        messages
    }

    fn can_open_windows(&self) -> bool {
        true
    }
}
