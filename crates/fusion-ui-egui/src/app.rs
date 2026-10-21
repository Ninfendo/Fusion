//! The Fusion app: a menu bar, the dock with its panels, and the links
//! editor.

mod host;
mod links_window;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use fusion_client_core::demo::DemoFeed;
use fusion_client_core::links::{LinkDraft, linked_panels};
use fusion_client_core::workspace::{default_layout, title_of};
use fusion_client_core::{FeedStatus, MarketFeed};
use fusion_egui_dock::{DockOutput, DockTree, to_node, to_tree};
use fusion_egui_theme::color;
use fusion_hub::{Bus, Fdc3Context, FromHub, LocalBus, ToHub};
use fusion_layout::{Layout, Node, PanelId, WindowId};
use fusion_theme::{Mode, Rgba, Theme};

use crate::panels::{Events, KINDS, Outgoing, PanelView, create};
use host::{FrameInput, Host, Move};

/// Where the layout is saved, as JSON.
pub const LAYOUT_KEY: &str = "fusion.layout";
/// Where the theme choice is saved.
pub const THEME_KEY: &str = "fusion.theme";

/// The id of the main window's dock.
fn dock_id() -> egui::Id {
    egui::Id::unique("fusion-dock")
}

/// The longest a window goes without drawing: the demo market moves, and
/// price flashes fade. A live feed also wakes the window when data comes.
const STEP: Duration = Duration::from_millis(100);

/// The colour of the market status in the menu bar.
fn status_colour(status: &FeedStatus, theme: &Theme) -> Rgba {
    match status {
        FeedStatus::Demo => theme.text_muted,
        FeedStatus::Connecting => theme.warning,
        FeedStatus::Live => theme.text,
        FeedStatus::Away(_) => theme.error,
    }
}

/// The saved form of a theme choice: `dark`, `light`, `dark+cb`, `light+cb`.
pub fn theme_to_text(mode: Mode, colour_blind: bool) -> String {
    let mode = match mode {
        Mode::Dark => "dark",
        Mode::Light => "light",
    };
    if colour_blind {
        format!("{mode}+cb")
    } else {
        mode.to_owned()
    }
}

/// Reads a saved theme choice. Anything unknown gives dark, not colour-blind.
pub fn theme_from_text(text: &str) -> (Mode, bool) {
    let (mode, colour_blind) = match text.strip_suffix("+cb") {
        Some(mode) => (mode, true),
        None => (text, false),
    };
    match mode {
        "light" => (Mode::Light, colour_blind),
        "dark" => (Mode::Dark, colour_blind),
        _ => (Mode::Dark, false),
    }
}

/// The Fusion app: one window of the workspace.
///
/// The app never changes the layout itself. It sends each change to the hub
/// through its [`Bus`] and takes the layout the hub sends back. With a
/// [`LocalBus`] the hub runs inside the app; with the launcher, in another
/// process.
pub struct FusionApp {
    bus: Box<dyn Bus>,
    /// The window this app shows.
    window: WindowId,
    /// The layout, as the hub last sent it.
    layout: Layout,
    tree: DockTree,
    views: BTreeMap<PanelId, Box<dyn PanelView>>,
    mode: Mode,
    colour_blind: bool,
    theme_dirty: bool,
    market: Box<dyn MarketFeed>,
    /// The market status last logged and shown.
    status: Option<FeedStatus>,
    events: Events,
    /// The panel under the pointer last frame; its linked panels get an
    /// outline.
    hovered: Option<PanelId>,
    links_open: bool,
    draft: LinkDraft,
    /// The hub asked this window to close.
    quit: bool,
    /// The window title last shown.
    title_shown: Option<String>,
}

impl FusionApp {
    /// Starts the app on its own, with the layout and theme saved last time
    /// if any.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let saved = |key| cc.storage.and_then(|s| s.get_string(key));
        Self::from_saved(saved(LAYOUT_KEY).as_deref(), saved(THEME_KEY).as_deref())
    }

    /// Starts the app on its own, from a saved layout and theme. A layout
    /// that does not load is replaced by the default one, and the log says
    /// why.
    pub fn from_saved(layout: Option<&str>, theme: Option<&str>) -> Self {
        let mut notice = None;
        let layout = match layout.map(Layout::from_json) {
            Some(Ok(layout)) => layout,
            Some(Err(err)) => {
                notice = Some(format!(
                    "The saved layout could not be loaded ({err}); using the default."
                ));
                default_layout()
            }
            None => default_layout(),
        };
        let mut app = Self::with_bus(Box::new(LocalBus::new(layout)), WindowId::MAIN, theme);
        if let Some(text) = notice {
            app.events.push(text);
        }
        app
    }

    /// Starts the app for one window, talking to the hub through `bus`.
    pub fn with_bus(bus: Box<dyn Bus>, window: WindowId, theme: Option<&str>) -> Self {
        let (mode, colour_blind) = theme.map_or((Mode::Dark, false), theme_from_text);
        let mut app = Self {
            bus,
            window,
            layout: Layout::new("Fusion"),
            tree: DockTree::empty(dock_id()),
            views: BTreeMap::new(),
            mode,
            colour_blind,
            theme_dirty: true,
            market: Box::new(DemoFeed::new(1)),
            status: None,
            events: Events::default(),
            hovered: None,
            links_open: false,
            draft: LinkDraft::default(),
            quit: false,
            title_shown: None,
        };
        app.pump();
        app
    }

    /// Shows market data from `market` instead of the demo market, for
    /// example a live feed from the view server.
    pub fn set_market(&mut self, market: Box<dyn MarketFeed>) {
        self.market = market;
        self.status = None;
    }

    /// The market status, as last shown.
    pub fn market_status(&self) -> Option<&FeedStatus> {
        self.status.as_ref()
    }

    /// The layout, as the hub last sent it.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The app's event log.
    pub fn events(&self) -> &Events {
        &self.events
    }

    /// The current theme.
    pub fn theme(&self) -> Theme {
        Theme::new(self.mode, self.colour_blind)
    }

    /// True once the hub asked this window to close.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Switches the theme; it is applied on the next frame.
    pub fn set_theme(&mut self, mode: Mode, colour_blind: bool) {
        if (mode, colour_blind) != (self.mode, self.colour_blind) {
            self.mode = mode;
            self.colour_blind = colour_blind;
            self.theme_dirty = true;
        }
    }

    /// Sends a message to the hub and takes in what comes back at once.
    fn send(&mut self, message: ToHub) {
        self.bus.send(message);
        self.pump();
    }

    /// Takes in everything the hub sent.
    fn pump(&mut self) {
        for message in self.bus.poll() {
            match message {
                FromHub::Layout { layout } => self.adopt(layout),
                FromHub::Deliver { panel, context } => {
                    if let Some(view) = self.views.get_mut(&panel) {
                        view.receive(&context.into(), &mut self.events);
                    }
                }
                FromHub::Notice { text } => self.events.push(text),
                FromHub::Quit => self.quit = true,
            }
        }
    }

    /// Takes a layout from the hub: views for this window's panels, and a
    /// new dock tree only if this window's tree changed, so an echo of the
    /// user's own edit does not disturb a drag. A window the layout no
    /// longer has, for example after its last panel popped in, closes.
    fn adopt(&mut self, layout: Layout) {
        let old_root = self.my_root().cloned();
        self.layout = layout;
        if self.layout.window(self.window).is_none() {
            self.quit = true;
        }
        let mine = self.layout.panels_in(self.window);
        self.views.retain(|id, _| mine.contains(id));
        for id in mine {
            if let Some(panel) = self.layout.panel(id) {
                let kind = panel.kind.clone();
                self.views.entry(id).or_insert_with(|| create(&kind));
            }
        }
        if self.my_root() != old_root.as_ref() {
            self.tree = to_tree(dock_id(), self.my_root());
        }
    }

    fn my_root(&self) -> Option<&Node> {
        self.layout
            .window(self.window)
            .and_then(|w| w.root.as_ref())
    }

    /// Adds a panel of `kind` to this window.
    pub fn add_panel(&mut self, kind: &str) {
        let title = title_of(kind);
        self.send(ToHub::AddPanel {
            window: self.window,
            kind: kind.to_owned(),
            title: title.to_owned(),
        });
        self.events.push(format!("Added {title}"));
    }

    /// Goes back to the default layout.
    pub fn reset_layout(&mut self) {
        self.send(ToHub::ReplaceLayout {
            layout: default_layout(),
        });
        self.events.push("Layout reset");
    }

    /// Sends the user's dock edits to the hub. The new tree is kept locally
    /// at once, so the hub's echo matches it.
    pub fn apply_dock_output(&mut self, output: DockOutput) {
        if !output.edited {
            return;
        }
        let root = to_node(&self.tree);
        if let Some(w) = self.layout.window_mut(self.window) {
            w.root = root.clone();
        }
        for id in &output.closed {
            if let Some(panel) = self.layout.panel(*id) {
                self.events.push(format!("Closed {}", panel.title));
            }
        }
        self.send(ToHub::WindowEdited {
            window: self.window,
            root,
            closed: output.closed,
        });
    }

    /// Takes what the user did inside the panels this frame: the hovered
    /// panel, clicks for last-focused links, colour groups and window moves.
    fn apply_frame(&mut self, frame: FrameInput) {
        self.hovered = frame.hovered;
        for panel in frame.pressed {
            self.send(ToHub::Focused { panel });
        }
        for (panel, colour) in frame.group_changes {
            if self.layout.panel(panel).is_none() {
                continue;
            }
            self.send(ToHub::SetGroup { panel, colour });
            let group = colour.map_or("no", |c| c.name());
            self.events
                .push(format!("Panel {}: {group} link group", panel.0));
        }
        for (panel, move_to) in frame.moves {
            self.send(match move_to {
                Move::Out => ToHub::PopOut { panel },
                Move::In => ToHub::PopIn { panel },
            });
        }
    }

    /// Sends each selection to the hub, which passes it to linked panels.
    pub fn deliver(&mut self, outbox: Vec<Outgoing>) {
        for out in outbox {
            self.send(ToHub::Select {
                from: out.from,
                context: Fdc3Context::from(&out.context),
                gesture: out.gesture,
            });
        }
    }

    /// This window's title from the layout, when it differs from the one
    /// shown. A popped-out window takes its panel's title.
    fn new_title(&mut self) -> Option<String> {
        let title = &self.layout.window(self.window)?.title;
        if self.title_shown.as_ref() == Some(title) {
            return None;
        }
        self.title_shown = Some(title.clone());
        self.title_shown.clone()
    }

    /// Brings the market up to date, names the books this window's panels
    /// show, and logs a change of market status.
    fn tick(&mut self, now: f64) {
        self.market.update(now);
        let books: BTreeSet<String> = self
            .views
            .values()
            .filter_map(|view| view.book())
            .map(str::to_owned)
            .collect();
        self.market.want_books(&books);
        let status = self.market.status();
        if self.status.as_ref() != Some(&status) {
            self.events.push(format!("Market: {status}"));
            self.status = Some(status);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("fusion-menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Panels", |ui| self.panels_menu(ui));
                ui.menu_button("Layout", |ui| self.layout_menu(ui));
                ui.menu_button("View", |ui| self.view_menu(ui));
                self.status_label(ui);
            });
        });
    }

    /// The market status, at the right end of the menu bar.
    fn status_label(&self, ui: &mut egui::Ui) {
        let Some(status) = &self.status else {
            return;
        };
        let tint = color(status_colour(status, &self.theme()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(status.to_string()).small().color(tint));
        });
    }

    fn panels_menu(&mut self, ui: &mut egui::Ui) {
        for kind in KINDS {
            if ui.button(format!("Add {}", kind.title)).clicked() {
                self.add_panel(kind.kind);
                ui.close();
            }
        }
    }

    fn layout_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Links...").clicked() {
            self.links_open = true;
            ui.close();
        }
        if ui.button("Reset layout").clicked() {
            self.reset_layout();
            ui.close();
        }
    }

    fn view_menu(&mut self, ui: &mut egui::Ui) {
        let (mut mode, mut colour_blind) = (self.mode, self.colour_blind);
        ui.radio_value(&mut mode, Mode::Dark, "Dark");
        ui.radio_value(&mut mode, Mode::Light, "Light");
        ui.checkbox(&mut colour_blind, "Colour-blind buy and sell");
        self.set_theme(mode, colour_blind);
    }

    fn dock(&mut self, ui: &mut egui::Ui, now: f64) {
        let theme = self.theme();
        let highlighted = self
            .hovered
            .map(|panel| linked_panels(&self.layout, panel))
            .unwrap_or_default();
        let mut frame = FrameInput::default();
        let mut outbox = Vec::new();
        let mut host = Host {
            layout: &self.layout,
            window: self.window,
            can_move: self.bus.can_open_windows(),
            views: &mut self.views,
            theme: &theme,
            market: &*self.market,
            now,
            events: &mut self.events,
            outbox: &mut outbox,
            highlighted: &highlighted,
            frame: &mut frame,
        };
        let output = fusion_egui_dock::show(&mut self.tree, ui, &mut host);
        self.apply_dock_output(output);
        self.apply_frame(frame);
        self.deliver(outbox);
    }

    fn links_window(&mut self, ctx: &egui::Context) {
        let mut open = self.links_open;
        egui::Window::new("Links")
            .open(&mut open)
            .show(ctx, |ui| self.links_editor(ui));
        self.links_open = open;
    }
}

impl eframe::App for FusionApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();
        if self.quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ui.input(|i| i.viewport().close_requested()) {
            self.send(ToHub::CloseWindow {
                window: self.window,
            });
        }
        if let Some(title) = self.new_title() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        if self.theme_dirty {
            fusion_egui_theme::apply(ui.ctx(), &self.theme());
            self.theme_dirty = false;
        }
        let now = ui.input(|i| i.time);
        self.tick(now);
        ui.ctx().request_repaint_after(STEP);
        self.menu_bar(ui);
        egui::CentralPanel::default().show(ui, |ui| self.dock(ui, now));
        self.links_window(&ui.ctx().clone());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // With a launcher, the launcher keeps the layout.
        if !self.bus.can_open_windows()
            && let Ok(json) = self.layout.to_json()
        {
            storage.set_string(LAYOUT_KEY, json);
        }
        storage.set_string(THEME_KEY, theme_to_text(self.mode, self.colour_blind));
    }
}

#[cfg(test)]
mod tests;
