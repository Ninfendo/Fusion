//! The links editor: lists the links and makes new ones.

use egui::Button;
use fusion_layout::{Gesture, Layout, Link, LinkMode, PanelId};

use fusion_hub::ToHub;

use super::FusionApp;

/// A panel as the editor names it: its title and its id, so two panels of
/// the same kind can be told apart.
pub(super) fn panel_label(layout: &Layout, id: PanelId) -> String {
    let title = layout.panel(id).map_or("Panel", |p| p.title.as_str());
    format!("{title} ({})", id.0)
}

/// The name of a mode, as the editor shows it.
pub(super) fn mode_name(mode: LinkMode) -> &'static str {
    match mode {
        LinkMode::All => "All",
        LinkMode::RoundRobin => "Round-robin",
        LinkMode::LastFocused => "Last focused",
    }
}

/// The name of a gesture, as the editor shows it.
pub(super) fn gesture_name(gesture: Option<Gesture>) -> &'static str {
    match gesture {
        None => "Click",
        Some(Gesture::Shift) => "Shift+click",
        Some(Gesture::Ctrl) => "Ctrl+click",
        Some(Gesture::Alt) => "Alt+click",
    }
}

/// One line describing a link: "Watchlist (1) -> L2 (2), All, Click".
pub(super) fn describe(layout: &Layout, link: &Link) -> String {
    let to: Vec<String> = link.to.iter().map(|id| panel_label(layout, *id)).collect();
    format!(
        "{} -> {}, {}, {}",
        panel_label(layout, link.from),
        to.join(" + "),
        mode_name(link.mode),
        gesture_name(link.gesture)
    )
}

const MODES: [LinkMode; 3] = [LinkMode::All, LinkMode::RoundRobin, LinkMode::LastFocused];
const GESTURES: [Option<Gesture>; 4] = [
    None,
    Some(Gesture::Shift),
    Some(Gesture::Ctrl),
    Some(Gesture::Alt),
];

impl FusionApp {
    pub(super) fn links_editor(&mut self, ui: &mut egui::Ui) {
        self.existing_links(ui);
        ui.separator();
        self.new_link(ui);
    }

    fn existing_links(&mut self, ui: &mut egui::Ui) {
        if self.layout.links.is_empty() {
            ui.label("No links yet.");
            return;
        }
        let mut remove = None;
        for (index, link) in self.layout.links.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(describe(&self.layout, link));
                if ui.button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.remove_link(index);
        }
    }

    fn new_link(&mut self, ui: &mut egui::Ui) {
        ui.label("New link");
        let panels: Vec<(PanelId, String)> = self
            .layout
            .panels
            .keys()
            .map(|id| (*id, panel_label(&self.layout, *id)))
            .collect();
        let from_text = self.draft.from.map_or("Choose a panel".to_owned(), |id| {
            panel_label(&self.layout, id)
        });
        egui::ComboBox::new("link-from", "From")
            .selected_text(from_text)
            .show_ui(ui, |ui| {
                for (id, label) in &panels {
                    ui.selectable_value(&mut self.draft.from, Some(*id), label);
                }
            });
        ui.label("To");
        for (id, label) in &panels {
            self.target_checkbox(ui, *id, label);
        }
        self.mode_and_gesture(ui);
        let link = self.draft.to_link();
        let clicked = ui
            .add_enabled(link.is_some(), Button::new("Add link"))
            .clicked();
        if let Some(link) = link.filter(|_| clicked) {
            self.add_link(link);
        }
    }

    fn target_checkbox(&mut self, ui: &mut egui::Ui, id: PanelId, label: &str) {
        if self.draft.from == Some(id) {
            return;
        }
        let mut on = self.draft.to.contains(&id);
        if ui.checkbox(&mut on, label).changed() {
            self.draft.toggle_target(id);
        }
    }

    fn mode_and_gesture(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::new("link-mode", "Mode")
            .selected_text(mode_name(self.draft.mode))
            .show_ui(ui, |ui| {
                for mode in MODES {
                    ui.selectable_value(&mut self.draft.mode, mode, mode_name(mode));
                }
            });
        egui::ComboBox::new("link-gesture", "Gesture")
            .selected_text(gesture_name(self.draft.gesture))
            .show_ui(ui, |ui| {
                for gesture in GESTURES {
                    ui.selectable_value(&mut self.draft.gesture, gesture, gesture_name(gesture));
                }
            });
    }

    /// Adds a link made in the editor. The link is checked here first, so
    /// the user hears at once if it is wrong; the hub checks it again.
    pub(super) fn add_link(&mut self, link: Link) {
        let text = describe(&self.layout, &link);
        match self.layout.clone().add_link(link.clone()) {
            Ok(()) => {
                self.send(ToHub::AddLink { link });
                self.draft = Default::default();
                self.events.push(format!("Added link: {text}"));
            }
            Err(err) => self.events.push(format!("Link refused: {err}")),
        }
    }

    /// Removes the link at `index`.
    pub(super) fn remove_link(&mut self, index: usize) {
        if let Some(link) = self.layout.links.get(index) {
            let text = describe(&self.layout, link);
            self.send(ToHub::RemoveLink { index });
            self.events.push(format!("Removed link: {text}"));
        }
    }
}
