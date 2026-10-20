//! The Fusion app: a menu bar, and the dock with its panels.

use std::collections::BTreeMap;
use std::time::Duration;

use fusion_client_core::demo::DemoMarket;
use fusion_egui_dock::{DockOutput, DockTree, PanelHost, to_node, to_tree};
use fusion_layout::{Child, Direction, Layout, Node, Panel, PanelId, WindowId};
use fusion_theme::{Mode, Theme};

use crate::panels::{Events, KINDS, PanelContext, PanelView, create};

/// Where the layout is saved, as JSON.
pub const LAYOUT_KEY: &str = "fusion.layout";
/// Where the theme choice is saved.
pub const THEME_KEY: &str = "fusion.theme";

/// The id of the main window's dock.
fn dock_id() -> egui::Id {
    egui::Id::unique("fusion-dock")
}

/// How often the demo market moves.
const STEP: Duration = Duration::from_millis(100);

/// The layout a first start shows: a watchlist on the left, an L2 ladder
/// and the log on the right.
pub fn default_layout() -> Layout {
    let mut layout = Layout::new("Fusion");
    for (id, kind) in (1..).zip(KINDS) {
        layout
            .panels
            .insert(PanelId(id), Panel::new(kind.kind, kind.title));
    }
    layout.next_panel_id = PanelId(KINDS.len() as u64 + 1);
    let tabs = |id| Node::tabs(vec![Node::panel(PanelId(id))]);
    layout.windows[0].root = Some(Node::Split {
        direction: Direction::Horizontal,
        children: vec![
            Child {
                share: 3.0,
                node: tabs(1),
            },
            Child {
                share: 2.0,
                node: Node::Split {
                    direction: Direction::Vertical,
                    children: vec![
                        Child {
                            share: 3.0,
                            node: tabs(2),
                        },
                        Child {
                            share: 1.0,
                            node: tabs(3),
                        },
                    ],
                },
            },
        ],
    });
    layout
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

/// The Fusion app.
pub struct FusionApp {
    layout: Layout,
    tree: DockTree,
    views: BTreeMap<PanelId, Box<dyn PanelView>>,
    mode: Mode,
    colour_blind: bool,
    theme_dirty: bool,
    market: DemoMarket,
    next_step_at: f64,
    events: Events,
}

impl FusionApp {
    /// Starts the app, with the layout and theme saved last time if any.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let saved = |key| cc.storage.and_then(|s| s.get_string(key));
        Self::from_saved(saved(LAYOUT_KEY).as_deref(), saved(THEME_KEY).as_deref())
    }

    /// Starts the app from a saved layout and theme. A layout that does not
    /// load is replaced by the default one, and the log says why.
    pub fn from_saved(layout: Option<&str>, theme: Option<&str>) -> Self {
        let mut events = Events::default();
        let layout = match layout.map(Layout::from_json) {
            Some(Ok(layout)) => layout,
            Some(Err(err)) => {
                events.push(format!(
                    "The saved layout could not be loaded ({err}); using the default."
                ));
                default_layout()
            }
            None => default_layout(),
        };
        let (mode, colour_blind) = theme.map_or((Mode::Dark, false), theme_from_text);
        let mut app = Self {
            tree: DockTree::empty(dock_id()),
            views: BTreeMap::new(),
            layout,
            mode,
            colour_blind,
            theme_dirty: true,
            market: DemoMarket::new(1),
            next_step_at: 0.0,
            events,
        };
        app.rebuild();
        app
    }

    /// The current layout.
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

    /// Switches the theme; it is applied on the next frame.
    pub fn set_theme(&mut self, mode: Mode, colour_blind: bool) {
        if (mode, colour_blind) != (self.mode, self.colour_blind) {
            self.mode = mode;
            self.colour_blind = colour_blind;
            self.theme_dirty = true;
        }
    }

    /// Adds a panel of `kind` to the main window.
    pub fn add_panel(&mut self, kind: &str) {
        let title = KINDS
            .iter()
            .find(|k| k.kind == kind)
            .map_or(kind, |k| k.title);
        if let Ok(id) = self
            .layout
            .add_panel(WindowId::MAIN, Panel::new(kind, title))
        {
            self.views.insert(id, create(kind));
            self.tree = to_tree(dock_id(), self.main_root());
            self.events.push(format!("Added {title}"));
        }
    }

    /// Goes back to the default layout.
    pub fn reset_layout(&mut self) {
        self.layout = default_layout();
        self.rebuild();
        self.events.push("Layout reset");
    }

    fn main_root(&self) -> Option<&Node> {
        self.layout
            .window(WindowId::MAIN)
            .and_then(|w| w.root.as_ref())
    }

    /// Makes the views and the drawable tree match the layout.
    fn rebuild(&mut self) {
        self.views = self
            .layout
            .panels
            .iter()
            .map(|(id, panel)| (*id, create(&panel.kind)))
            .collect();
        self.tree = to_tree(dock_id(), self.main_root());
    }

    /// Takes the user's dock edits into the layout.
    pub fn apply_dock_output(&mut self, output: DockOutput) {
        if !output.edited {
            return;
        }
        for id in output.closed {
            if let Some(panel) = self.layout.panels.remove(&id) {
                self.events.push(format!("Closed {}", panel.title));
            }
            self.views.remove(&id);
        }
        let root = to_node(&self.tree);
        if let Some(main) = self.layout.window_mut(WindowId::MAIN) {
            main.root = root;
        }
    }

    /// Moves the demo market when it is time.
    fn tick(&mut self, now: f64) {
        if now >= self.next_step_at {
            self.market.step();
            self.next_step_at = now + STEP.as_secs_f64();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("fusion-menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Panels", |ui| self.panels_menu(ui));
                ui.menu_button("Layout", |ui| self.layout_menu(ui));
                ui.menu_button("View", |ui| self.view_menu(ui));
            });
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
        let mut host = Host {
            layout: &self.layout,
            views: &mut self.views,
            theme: &theme,
            market: &self.market,
            now,
            events: &mut self.events,
        };
        let output = fusion_egui_dock::show(&mut self.tree, ui, &mut host);
        self.apply_dock_output(output);
    }
}

impl eframe::App for FusionApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.theme_dirty {
            fusion_egui_theme::apply(ui.ctx(), &self.theme());
            self.theme_dirty = false;
        }
        let now = ui.input(|i| i.time);
        self.tick(now);
        ui.ctx().request_repaint_after(STEP);
        self.menu_bar(ui);
        egui::CentralPanel::default().show(ui, |ui| self.dock(ui, now));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = self.layout.to_json() {
            storage.set_string(LAYOUT_KEY, json);
        }
        storage.set_string(THEME_KEY, theme_to_text(self.mode, self.colour_blind));
    }
}

/// Lets the dock draw panels from the app's views.
struct Host<'a> {
    layout: &'a Layout,
    views: &'a mut BTreeMap<PanelId, Box<dyn PanelView>>,
    theme: &'a Theme,
    market: &'a DemoMarket,
    now: f64,
    events: &'a mut Events,
}

impl PanelHost for Host<'_> {
    fn title(&self, id: PanelId) -> String {
        self.layout
            .panel(id)
            .map_or_else(|| format!("Panel {}", id.0), |p| p.title.clone())
    }

    fn ui(&mut self, ui: &mut egui::Ui, id: PanelId) {
        let mut cx = PanelContext {
            theme: self.theme,
            market: self.market,
            now: self.now,
            events: self.events,
        };
        if let Some(view) = self.views.get_mut(&id) {
            view.ui(ui, &mut cx);
        }
    }
}

#[cfg(test)]
mod tests;
