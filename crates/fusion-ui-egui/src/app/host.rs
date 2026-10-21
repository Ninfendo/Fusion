//! Lets the dock draw panels from the app's views, and collects what the
//! user did inside them.

use std::collections::BTreeMap;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Stroke, StrokeKind, WidgetText};
use fusion_client_core::MarketData;
use fusion_egui_dock::PanelHost;
use fusion_egui_theme::color;
use fusion_layout::{Layout, LinkColour, PanelId, WindowId};
use fusion_theme::Theme;

use crate::panels::{Events, Outgoing, PanelContext, PanelView};

/// What the user did inside the panels in one frame.
#[derive(Debug, Default)]
pub(super) struct FrameInput {
    /// The panel under the pointer.
    pub hovered: Option<PanelId>,
    /// Panels the user pressed a mouse button in.
    pub pressed: Vec<PanelId>,
    /// Colour group changes from the panel headers.
    pub group_changes: Vec<(PanelId, Option<LinkColour>)>,
    /// Pop out and pop in clicks from the panel headers.
    pub moves: Vec<(PanelId, Move)>,
}

/// Where the user wants a panel to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Move {
    /// Into a window of its own.
    Out,
    /// Back to where it came from.
    In,
}

impl Move {
    /// The button text.
    pub fn label(self) -> &'static str {
        match self {
            Self::Out => "Pop out",
            Self::In => "Pop in",
        }
    }
}

/// The moves a panel header offers. A panel can pop out unless it is
/// already alone in a window of its own; it can pop in when it is outside
/// the main window or remembers where it came from.
pub(super) fn offered_moves(layout: &Layout, window: WindowId, panel: PanelId) -> Vec<Move> {
    let main = window == WindowId::MAIN;
    let popped = layout.panel(panel).is_some_and(|p| p.origin.is_some());
    let mut moves = Vec::new();
    if main || layout.panels_in(window).len() > 1 {
        moves.push(Move::Out);
    }
    if !main || popped {
        moves.push(Move::In);
    }
    moves
}

/// The colour of a link group's square.
pub fn group_colour(colour: LinkColour) -> Color32 {
    match colour {
        LinkColour::Red => Color32::from_rgb(0xe5, 0x48, 0x4d),
        LinkColour::Green => Color32::from_rgb(0x30, 0xa4, 0x6c),
        LinkColour::Blue => Color32::from_rgb(0x3e, 0x63, 0xdd),
        LinkColour::Yellow => Color32::from_rgb(0xff, 0xc5, 0x3d),
    }
}

/// A tab title, followed by its link group in the group's colour:
/// "L2 [Red]". Plain ASCII, so every font can draw it.
pub fn title_text(title: &str, colour: Option<LinkColour>) -> WidgetText {
    let Some(colour) = colour else {
        return title.into();
    };
    let mut job = LayoutJob::default();
    job.append(
        title,
        0.0,
        TextFormat {
            // The placeholder becomes the tab's normal text colour.
            color: Color32::PLACEHOLDER,
            ..TextFormat::default()
        },
    );
    job.append(
        &format!(" [{}]", colour.name()),
        0.0,
        TextFormat {
            color: group_colour(colour),
            ..TextFormat::default()
        },
    );
    job.into()
}

pub(super) fn group_name(colour: Option<LinkColour>) -> &'static str {
    colour.map_or("None", LinkColour::name)
}

pub(super) struct Host<'a> {
    pub layout: &'a Layout,
    /// The window these panels are in.
    pub window: WindowId,
    /// True if the hub can open windows, so panels can pop out.
    pub can_move: bool,
    pub views: &'a mut BTreeMap<PanelId, Box<dyn PanelView>>,
    pub theme: &'a Theme,
    pub market: &'a dyn MarketData,
    pub now: f64,
    pub events: &'a mut Events,
    pub outbox: &'a mut Vec<Outgoing>,
    /// Panels to outline: the ones the hovered panel links to.
    pub highlighted: &'a [PanelId],
    pub frame: &'a mut FrameInput,
}

impl Host<'_> {
    /// The panel header: the link group picker, then pop out and pop in.
    fn header(&mut self, ui: &mut egui::Ui, id: PanelId) {
        ui.horizontal(|ui| {
            self.group_chip(ui, id);
            if self.can_move {
                self.move_buttons(ui, id);
            }
        });
    }

    /// Pop out and pop in, where they work.
    fn move_buttons(&mut self, ui: &mut egui::Ui, id: PanelId) {
        for move_to in offered_moves(self.layout, self.window, id) {
            if ui.button(move_to.label()).clicked() {
                self.frame.moves.push((id, move_to));
            }
        }
    }

    /// The link group picker.
    fn group_chip(&mut self, ui: &mut egui::Ui, id: PanelId) {
        let current = self.layout.groups.get(&id).copied();
        let mut chosen = current;
        egui::ComboBox::new(ui.scope_id().with("link-group"), "Link group")
            .selected_text(group_name(chosen))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "None");
                for colour in LinkColour::ALL {
                    ui.selectable_value(&mut chosen, Some(colour), colour.name());
                }
            });
        if chosen != current {
            self.frame.group_changes.push((id, chosen));
        }
    }
}

impl PanelHost for Host<'_> {
    fn title(&self, id: PanelId) -> WidgetText {
        let title = self
            .layout
            .panel(id)
            .map_or_else(|| format!("Panel {}", id.0), |p| p.title.clone());
        title_text(&title, self.layout.groups.get(&id).copied())
    }

    fn ui(&mut self, ui: &mut egui::Ui, id: PanelId) {
        let area = ui.max_rect();
        self.header(ui, id);
        let mut cx = PanelContext {
            panel: id,
            theme: self.theme,
            market: self.market,
            now: self.now,
            events: self.events,
            outbox: self.outbox,
        };
        if let Some(view) = self.views.get_mut(&id) {
            view.ui(ui, &mut cx);
        }
        if ui.rect_contains_pointer(area) {
            self.frame.hovered = Some(id);
            if ui.input(|i| i.pointer.any_pressed()) {
                self.frame.pressed.push(id);
            }
        }
        if self.highlighted.contains(&id) {
            let stroke = Stroke::new(2.0, color(self.theme.accent));
            ui.painter()
                .rect_stroke(area.shrink(1.0), 2.0, stroke, StrokeKind::Inside);
        }
    }
}
