use std::collections::BTreeMap;

use eframe::Storage as _;
use egui_kittest::{Harness, kittest::Queryable as _};
use fusion_client_core::links::Context;
use fusion_egui_dock::PanelHost as _;
use fusion_layout::{ContextType, Gesture, Link, LinkColour, LinkMode};

use std::cell::RefCell;
use std::rc::Rc;

use fusion_hub::{Hub, InstrumentIds};

use super::host::{FrameInput, Host, Move, group_colour, group_name, offered_moves, title_text};
use super::links_window;
use crate::panels::Outgoing;

use super::*;

/// eframe storage in memory, for save and restore tests.
#[derive(Default)]
struct MemoryStorage(BTreeMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

fn harness() -> Harness<'static, FusionApp> {
    let mut harness = Harness::new_eframe(|cc| FusionApp::new(cc));
    harness.run_steps(3);
    harness
}

fn has_event(app: &FusionApp, text: &str) -> bool {
    app.events().lines().any(|line| line == text)
}

// --- Logic -------------------------------------------------------------------

#[test]
fn the_default_layout_is_valid_and_has_every_kind() {
    let layout = default_layout();
    assert_eq!(layout.validate(), Ok(()));
    let kinds: Vec<&str> = layout.panels.values().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds, ["watchlist", "l2", "log"]);
}

#[test]
fn theme_choices_round_trip() {
    for mode in [Mode::Dark, Mode::Light] {
        for colour_blind in [false, true] {
            assert_eq!(
                theme_from_text(&theme_to_text(mode, colour_blind)),
                (mode, colour_blind)
            );
        }
    }
    assert_eq!(theme_to_text(Mode::Light, true), "light+cb");
    assert_eq!(theme_from_text("purple"), (Mode::Dark, false));
    assert_eq!(theme_from_text("purple+cb"), (Mode::Dark, false));
}

#[test]
fn a_first_start_shows_the_default_layout_in_dark() {
    let app = FusionApp::from_saved(None, None);
    assert_eq!(app.layout(), &default_layout());
    assert_eq!(app.theme(), Theme::dark());
    assert!(app.events().is_empty());
}

#[test]
fn a_broken_saved_layout_falls_back_and_says_so() {
    let app = FusionApp::from_saved(Some("{ broken"), Some("light"));
    assert_eq!(app.layout(), &default_layout());
    assert_eq!(app.theme(), Theme::light());
    assert!(
        app.events()
            .lines()
            .any(|l| l.starts_with("The saved layout could not be loaded"))
    );
}

#[test]
fn a_saved_layout_is_used() {
    let mut layout = default_layout();
    layout
        .close_panel(PanelId(3))
        .expect("the log panel exists");
    let json = layout.to_json().expect("serialises");
    let app = FusionApp::from_saved(Some(&json), None);
    assert_eq!(app.layout(), &layout);
}

#[test]
fn adding_a_panel_updates_layout_views_and_dock() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_panel("l2");
    let id = PanelId(4);
    assert_eq!(app.layout().panel(id).map(|p| p.title.as_str()), Some("L2"));
    assert!(app.views.contains_key(&id));
    assert!(
        to_node(&app.tree)
            .map(|n| n.panel_ids())
            .unwrap_or_default()
            .contains(&id)
    );
    assert!(has_event(&app, "Added L2"));
    assert_eq!(app.layout().validate(), Ok(()));
}

#[test]
fn an_unknown_kind_gets_its_kind_as_title() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_panel("heatmap");
    assert_eq!(
        app.layout().panel(PanelId(4)).map(|p| p.title.as_str()),
        Some("heatmap")
    );
}

#[test]
fn reset_brings_back_the_default_layout() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_panel("log");
    app.reset_layout();
    assert_eq!(app.layout(), &default_layout());
    assert_eq!(app.views.len(), 3);
    assert!(has_event(&app, "Layout reset"));
}

#[test]
fn closing_a_tab_removes_the_panel() {
    let mut app = FusionApp::from_saved(None, None);
    // What the dock holds after the user closed the watchlist tab.
    let after = Node::Split {
        direction: Direction::Vertical,
        children: vec![
            Child {
                share: 3.0,
                node: Node::tabs(vec![Node::panel(PanelId(2))]),
            },
            Child {
                share: 1.0,
                node: Node::tabs(vec![Node::panel(PanelId(3))]),
            },
        ],
    };
    app.tree = to_tree(dock_id(), Some(&after));
    app.apply_dock_output(DockOutput {
        edited: true,
        closed: vec![PanelId(1)],
    });
    assert!(app.layout().panel(PanelId(1)).is_none());
    assert!(!app.views.contains_key(&PanelId(1)));
    assert!(has_event(&app, "Closed Watchlist"));
    assert_eq!(
        app.layout()
            .window(WindowId::MAIN)
            .and_then(|w| w.root.clone()),
        Some(after)
    );
    assert_eq!(app.layout().validate(), Ok(()));
}

#[test]
fn an_unedited_frame_changes_nothing() {
    let mut app = FusionApp::from_saved(None, None);
    app.tree = DockTree::empty(dock_id());
    app.apply_dock_output(DockOutput {
        edited: false,
        closed: vec![PanelId(1)],
    });
    assert_eq!(app.layout(), &default_layout());
}

#[test]
fn the_theme_changes_only_when_it_differs() {
    let mut app = FusionApp::from_saved(None, None);
    app.theme_dirty = false;
    app.set_theme(Mode::Dark, false);
    assert!(!app.theme_dirty);
    app.set_theme(Mode::Light, true);
    assert!(app.theme_dirty);
    assert_eq!(app.theme(), Theme::new(Mode::Light, true));
}

#[test]
fn the_market_steps_every_100_milliseconds() {
    let mut app = FusionApp::from_saved(None, None);
    app.tick(0.0);
    assert_eq!(app.market.steps(), 1);
    app.tick(0.05);
    assert_eq!(app.market.steps(), 1);
    app.tick(0.1);
    assert_eq!(app.market.steps(), 2);
}

#[test]
fn save_and_restore_give_the_same_app_state() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_panel("watchlist");
    app.set_theme(Mode::Light, true);
    let mut storage = MemoryStorage::default();
    eframe::App::save(&mut app, &mut storage);
    let restored = FusionApp::from_saved(
        storage.get_string(LAYOUT_KEY).as_deref(),
        storage.get_string(THEME_KEY).as_deref(),
    );
    assert_eq!(restored.layout(), app.layout());
    assert_eq!(restored.theme(), app.theme());
}

#[test]
fn a_panel_without_a_view_draws_nothing() {
    let mut app = FusionApp::from_saved(None, None);
    app.views.clear();
    let theme = app.theme();
    let mut outbox = Vec::new();
    let mut frame = FrameInput::default();
    let mut host = Host {
        layout: &app.layout,
        window: WindowId::MAIN,
        can_move: false,
        views: &mut app.views,
        theme: &theme,
        market: &app.market,
        now: 0.0,
        events: &mut app.events,
        outbox: &mut outbox,
        highlighted: &[],
        frame: &mut frame,
    };
    assert_eq!(host.title(PanelId(9)).text(), "Panel 9");
    let mut harness = Harness::new_ui(|ui| host.ui(ui, PanelId(1)));
    harness.run();
}

// --- Links ------------------------------------------------------------------------

#[test]
fn a_grouped_title_colours_only_the_group_name() {
    let egui::WidgetText::LayoutJob(job) = title_text("L2", Some(LinkColour::Blue)) else {
        panic!("a grouped title is a layout job");
    };
    assert_eq!(job.sections.len(), 2);
    assert_eq!(job.sections[0].format.color, egui::Color32::PLACEHOLDER);
    assert_eq!(job.sections[1].format.color, group_colour(LinkColour::Blue));
}

#[test]
fn the_group_picker_names_the_group() {
    assert_eq!(group_name(None), "None");
    assert_eq!(group_name(Some(LinkColour::Yellow)), "Yellow");
}

#[test]
fn titles_carry_the_group_colour() {
    assert_eq!(title_text("L2", None).text(), "L2");
    assert_eq!(title_text("L2", Some(LinkColour::Red)).text(), "L2 [Red]");
    let colours: Vec<_> = LinkColour::ALL.iter().map(|c| group_colour(*c)).collect();
    for (i, a) in colours.iter().enumerate() {
        assert!(colours[i + 1..].iter().all(|b| a != b), "colours differ");
    }
}

#[test]
fn a_selection_reaches_only_linked_panels() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_panel("l2");
    app.deliver(vec![Outgoing {
        from: PanelId(1),
        context: Context::instrument("ETHUSDT"),
        gesture: None,
    }]);
    assert!(has_event(&app, "L2: showing ETHUSDT (linked)"));
    let shown: Vec<&str> = app
        .events()
        .lines()
        .filter(|l| l.contains("(linked)"))
        .collect();
    assert_eq!(shown.len(), 1, "only panel 2 follows, not the new L2 (4)");
}

#[test]
fn a_frame_records_hover_focus_and_group_changes() {
    let mut app = FusionApp::from_saved(None, None);
    app.apply_frame(FrameInput {
        hovered: Some(PanelId(1)),
        pressed: vec![PanelId(2)],
        group_changes: vec![
            (PanelId(3), Some(LinkColour::Blue)),
            (PanelId(9), Some(LinkColour::Red)),
        ],
        moves: Vec::new(),
    });
    assert_eq!(app.hovered, Some(PanelId(1)));
    assert_eq!(
        app.layout().groups.get(&PanelId(3)),
        Some(&LinkColour::Blue)
    );
    assert!(has_event(&app, "Panel 3: Blue link group"));
    assert!(
        !app.layout().groups.contains_key(&PanelId(9)),
        "unknown panel ignored"
    );
    assert!(!has_event(&app, "Panel 9: Red link group"));
    app.apply_frame(FrameInput {
        group_changes: vec![(PanelId(3), None)],
        ..FrameInput::default()
    });
    assert!(has_event(&app, "Panel 3: no link group"));
}

#[test]
fn links_are_described_for_people() {
    let layout = default_layout();
    let mut link = layout.links[0].clone();
    assert_eq!(
        links_window::describe(&layout, &link),
        "Watchlist (1) -> L2 (2), All, Click"
    );
    link.mode = LinkMode::RoundRobin;
    link.gesture = Some(Gesture::Shift);
    link.to.push(PanelId(3));
    assert_eq!(
        links_window::describe(&layout, &link),
        "Watchlist (1) -> L2 (2) + Log (3), Round-robin, Shift+click"
    );
    assert_eq!(
        links_window::mode_name(LinkMode::LastFocused),
        "Last focused"
    );
    assert_eq!(
        links_window::gesture_name(Some(Gesture::Ctrl)),
        "Ctrl+click"
    );
    assert_eq!(links_window::gesture_name(Some(Gesture::Alt)), "Alt+click");
    assert_eq!(links_window::panel_label(&layout, PanelId(9)), "Panel (9)");
}

#[test]
fn a_refused_link_is_logged() {
    let mut app = FusionApp::from_saved(None, None);
    app.add_link(Link {
        from: PanelId(1),
        context: ContextType::instrument(),
        to: vec![PanelId(9)],
        mode: LinkMode::All,
        gesture: None,
    });
    assert!(
        app.events()
            .lines()
            .any(|l| l == "Link refused: there is no panel 9")
    );
    assert_eq!(app.layout().links.len(), 1);
}

#[test]
fn closing_a_panel_drops_its_links() {
    let mut app = FusionApp::from_saved(None, None);
    app.tree = to_tree(dock_id(), Some(&Node::tabs(vec![Node::panel(PanelId(1))])));
    app.apply_dock_output(DockOutput {
        edited: true,
        closed: vec![PanelId(2), PanelId(3)],
    });
    assert!(app.layout().links.is_empty());
    assert_eq!(app.layout().validate(), Ok(()));
}

#[test]
fn clicking_a_watchlist_row_moves_the_linked_ladder() {
    let mut harness = harness();
    harness.get_by_label("ETHUSDT").click();
    harness.run_steps(3);
    assert!(has_event(harness.state(), "L2: showing ETHUSDT (linked)"));
}

#[test]
fn a_shift_click_follows_the_shift_link_only() {
    let mut harness = harness();
    harness.state_mut().add_panel("l2");
    harness.state_mut().add_link(Link {
        from: PanelId(1),
        context: ContextType::instrument(),
        to: vec![PanelId(4)],
        mode: LinkMode::All,
        gesture: Some(Gesture::Shift),
    });
    harness.run_steps(3);
    harness
        .get_by_label("SOLUSDT")
        .click_modifiers(egui::Modifiers::SHIFT);
    harness.run_steps(3);
    let linked: Vec<String> = harness
        .state()
        .events()
        .lines()
        .filter(|l| l.contains("SOLUSDT (linked)"))
        .map(str::to_owned)
        .collect();
    assert_eq!(linked.len(), 1, "only the new L2 follows: {linked:?}");
}

#[test]
fn the_links_editor_adds_and_removes_links() {
    let mut harness = harness();
    harness.state_mut().links_open = true;
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label("Watchlist (1) -> L2 (2), All, Click")
            .is_some()
    );
    harness.get_by_label("Remove").click();
    harness.run_steps(3);
    assert!(harness.state().layout().links.is_empty());
    assert!(harness.query_by_label("No links yet.").is_some());

    harness.get_by_label("From").click();
    harness.run_steps(2);
    // The From list item (a button), not the target checkbox with the same label.
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Watchlist (1)")
        .click();
    harness.run_steps(2);
    harness.get_by_label("Log (3)").click();
    harness.run_steps(2);
    harness.get_by_label("Mode").click();
    harness.run_steps(2);
    harness.get_by_label("Round-robin").click();
    harness.run_steps(2);
    harness.get_by_label("Gesture").click();
    harness.run_steps(2);
    harness.get_by_label("Alt+click").click();
    harness.run_steps(2);
    harness.get_by_label("Add link").click();
    harness.run_steps(3);
    let links = &harness.state().layout().links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].to, [PanelId(3)]);
    assert_eq!(links[0].mode, LinkMode::RoundRobin);
    assert_eq!(links[0].gesture, Some(Gesture::Alt));
}

#[test]
fn the_layout_menu_opens_the_links_editor() {
    let mut harness = harness();
    harness.get_by_label("Layout").click();
    harness.run_steps(2);
    harness.get_by_label("Links...").click();
    harness.run_steps(2);
    assert!(harness.state().links_open);
    assert!(harness.query_by_label("New link").is_some());
}

#[test]
fn a_panel_header_changes_the_link_group() {
    let mut harness = harness();
    let pickers = harness.get_all_by_label("Link group").count();
    assert_eq!(pickers, 3, "one picker per panel");
    harness
        .get_all_by_label("Link group")
        .next()
        .expect("a picker")
        .click();
    harness.run_steps(2);
    harness.get_by_label("Green").click();
    harness.run_steps(3);
    assert_eq!(harness.state().layout().groups.len(), 1);
}

// --- The running app -------------------------------------------------------------

#[test]
fn the_app_shows_the_three_panels_with_data() {
    let harness = harness();
    for label in [
        "Watchlist",
        "L2",
        "Log",
        "Bid",
        "Chg %",
        "ETHUSDT",
        "Panels",
        "View",
    ] {
        assert!(
            harness.query_by_label(label).is_some(),
            "{label} is missing"
        );
    }
}

#[test]
fn the_panels_menu_adds_a_panel() {
    let mut harness = harness();
    harness.get_by_label("Panels").click();
    harness.run_steps(2);
    harness.get_by_label("Add Log").click();
    harness.run_steps(2);
    assert_eq!(harness.state().layout().panels.len(), 4);
    assert!(has_event(harness.state(), "Added Log"));
}

#[test]
fn the_layout_menu_resets() {
    let mut harness = harness();
    harness.state_mut().add_panel("log");
    harness.run_steps(2);
    harness.get_by_label("Layout").click();
    harness.run_steps(2);
    harness.get_by_label("Reset layout").click();
    harness.run_steps(2);
    assert_eq!(harness.state().layout(), &default_layout());
}

#[test]
fn the_view_menu_switches_to_light() {
    let mut harness = harness();
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label("Light").click();
    harness.run_steps(2);
    // A click closes the menu, so open it again.
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label("Colour-blind buy and sell").click();
    harness.run_steps(2);
    assert_eq!(harness.state().theme(), Theme::new(Mode::Light, true));
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);
}

#[test]
fn clicking_a_watchlist_row_selects_the_instrument() {
    let mut harness = harness();
    harness.get_by_label("ETHUSDT").click();
    harness.run_steps(2);
    assert!(has_event(harness.state(), "Watchlist: ETHUSDT selected"));
}

#[test]
fn the_ladder_starts_on_the_first_symbol() {
    let harness = harness();
    assert!(has_event(harness.state(), "L2: showing BTCUSDT"));
}

#[test]
fn an_unknown_panel_kind_shows_a_placeholder() {
    let mut layout = default_layout();
    if let Some(panel) = layout.panels.get_mut(&PanelId(1)) {
        panel.kind = "heatmap".to_owned();
    }
    let json = layout.to_json().expect("serialises");
    let mut harness = Harness::new_eframe(move |_cc| FusionApp::from_saved(Some(&json), None));
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label(
                "Unknown panel kind \"heatmap\". This build cannot show it; it stays in your layout."
            )
            .is_some()
    );
}

// --- Hub -------------------------------------------------------------------------

/// One hub shared by several windows, as the launcher will run it.
#[derive(Clone)]
struct SharedHub {
    hub: Rc<RefCell<Hub>>,
    inboxes: Rc<RefCell<BTreeMap<WindowId, Vec<FromHub>>>>,
}

impl SharedHub {
    fn new(layout: Layout) -> Self {
        Self {
            hub: Rc::new(RefCell::new(Hub::new(layout))),
            inboxes: Rc::default(),
        }
    }

    /// A bus for one window. Like a transport that just connected, it says
    /// hello first.
    fn bus(&self, window: WindowId) -> Box<dyn Bus> {
        let mut bus = HubBus {
            shared: self.clone(),
            window,
        };
        bus.send(ToHub::Hello { window });
        Box::new(bus)
    }
}

struct HubBus {
    shared: SharedHub,
    window: WindowId,
}

impl Bus for HubBus {
    fn send(&mut self, message: ToHub) {
        let mut hub = self.shared.hub.borrow_mut();
        // A window that this message closes still hears about it, as it
        // does from the launcher before its process ends.
        let mut windows = hub.window_ids();
        let reply = hub.handle(self.window, message);
        windows.extend(hub.window_ids());
        windows.sort();
        windows.dedup();
        let mut inboxes = self.shared.inboxes.borrow_mut();
        for out in reply.messages {
            for window in windows.iter().filter(|w| out.to.reaches(**w)) {
                inboxes
                    .entry(*window)
                    .or_default()
                    .push(out.message.clone());
            }
        }
    }

    fn poll(&mut self) -> Vec<FromHub> {
        self.shared
            .inboxes
            .borrow_mut()
            .remove(&self.window)
            .unwrap_or_default()
    }

    fn can_open_windows(&self) -> bool {
        true
    }
}

/// A bus that records what the app sends and hands it scripted replies.
#[derive(Clone, Default)]
struct ScriptBus {
    sent: Rc<RefCell<Vec<ToHub>>>,
    replies: Rc<RefCell<Vec<FromHub>>>,
}

impl Bus for ScriptBus {
    fn send(&mut self, message: ToHub) {
        self.sent.borrow_mut().push(message);
    }

    fn poll(&mut self) -> Vec<FromHub> {
        std::mem::take(&mut *self.replies.borrow_mut())
    }

    fn can_open_windows(&self) -> bool {
        true
    }
}

fn eth() -> Fdc3Context {
    Fdc3Context::Instrument {
        id: InstrumentIds {
            ticker: "ETHUSDT".to_owned(),
        },
    }
}

fn pop_out(app: &mut FusionApp, panel: PanelId) {
    app.apply_frame(FrameInput {
        moves: vec![(panel, Move::Out)],
        ..FrameInput::default()
    });
}

#[test]
fn a_selection_reaches_only_the_linked_panel_in_another_window() {
    let hub = SharedHub::new(default_layout());
    let mut main = FusionApp::with_bus(hub.bus(WindowId::MAIN), WindowId::MAIN, None);
    main.add_panel("l2");
    pop_out(&mut main, PanelId(2));
    assert!(!main.views.contains_key(&PanelId(2)), "panel 2 left");
    let window = main.layout().window_of(PanelId(2)).expect("popped out");
    let mut other = FusionApp::with_bus(hub.bus(window), window, None);
    assert_eq!(
        other.views.keys().copied().collect::<Vec<_>>(),
        [PanelId(2)]
    );

    main.deliver(vec![Outgoing {
        from: PanelId(1),
        context: Context::instrument("ETHUSDT"),
        gesture: None,
    }]);
    other.pump();
    assert!(has_event(&other, "L2: showing ETHUSDT (linked)"));
    assert!(
        !main.events().lines().any(|l| l.contains("(linked)")),
        "the L2 in the main window is not linked"
    );

    // The other window's process dies and starts again.
    drop(other);
    let restarted = FusionApp::with_bus(hub.bus(window), window, None);
    assert!(restarted.views.contains_key(&PanelId(2)));
    assert!(has_event(&restarted, "L2: showing ETHUSDT (linked)"));
}

#[test]
fn pop_in_brings_a_panel_back() {
    let hub = SharedHub::new(default_layout());
    let mut main = FusionApp::with_bus(hub.bus(WindowId::MAIN), WindowId::MAIN, None);
    pop_out(&mut main, PanelId(3));
    let window = main.layout().window_of(PanelId(3)).expect("popped out");
    let mut other = FusionApp::with_bus(hub.bus(window), window, None);
    other.apply_frame(FrameInput {
        moves: vec![(PanelId(3), Move::In)],
        ..FrameInput::default()
    });
    main.pump();
    assert_eq!(main.layout().windows.len(), 1);
    assert!(main.views.contains_key(&PanelId(3)));
    assert!(other.quit_requested(), "an empty window closes");
}

#[test]
fn headers_offer_the_moves_that_work() {
    let mut layout = default_layout();
    assert_eq!(
        offered_moves(&layout, WindowId::MAIN, PanelId(1)),
        [Move::Out]
    );
    let window = layout.pop_out(PanelId(2)).expect("pops out");
    assert_eq!(offered_moves(&layout, window, PanelId(2)), [Move::In]);
    layout.pop_in(PanelId(2)).expect("pops in");
    let window = layout.pop_out(PanelId(2)).expect("pops out");
    layout
        .add_panel(window, Panel::new("log", "Log"))
        .expect("added");
    assert_eq!(
        offered_moves(&layout, window, PanelId(2)),
        [Move::Out, Move::In]
    );
    let mut alone = default_layout();
    alone.pop_out(PanelId(2)).expect("pops out");
    alone.pop_out(PanelId(3)).expect("pops out");
    assert_eq!(
        offered_moves(&alone, WindowId::MAIN, PanelId(1)),
        [Move::Out],
        "the last panel of the main window can still pop out"
    );
    let mut handmade = default_layout();
    let window = handmade.pop_out(PanelId(2)).expect("pops out");
    if let Some(panel) = handmade.panels.get_mut(&PanelId(2)) {
        panel.origin = None;
    }
    assert_eq!(
        offered_moves(&handmade, window, PanelId(2)),
        [Move::In],
        "a panel outside the main window can always pop in"
    );
    assert_eq!(Move::Out.label(), "Pop out");
    assert_eq!(Move::In.label(), "Pop in");
}

#[test]
fn the_app_takes_what_the_hub_sends() {
    let bus = ScriptBus::default();
    let mut app = FusionApp::with_bus(Box::new(bus.clone()), WindowId::MAIN, None);
    bus.replies.borrow_mut().extend([
        FromHub::Layout {
            layout: default_layout(),
        },
        FromHub::Deliver {
            panel: PanelId(9),
            context: eth(),
        },
        FromHub::Deliver {
            panel: PanelId(2),
            context: eth(),
        },
        FromHub::Notice {
            text: "Refused: hello".to_owned(),
        },
    ]);
    app.pump();
    assert_eq!(app.views.len(), 3);
    assert!(has_event(&app, "L2: showing ETHUSDT (linked)"));
    assert!(has_event(&app, "Refused: hello"));
    assert!(!app.quit_requested());
}

#[test]
fn an_echo_of_the_users_edit_keeps_the_dock() {
    let hub = SharedHub::new(default_layout());
    let mut app = FusionApp::with_bus(hub.bus(WindowId::MAIN), WindowId::MAIN, None);
    let tree_before = app.tree.clone();
    let layout = app.layout().clone();
    app.adopt(layout);
    assert_eq!(app.tree, tree_before);
}

#[test]
fn panel_headers_pop_out_when_the_hub_can_open_windows() {
    let hub = SharedHub::new(default_layout());
    let mut harness =
        Harness::new_eframe(|_| FusionApp::with_bus(hub.bus(WindowId::MAIN), WindowId::MAIN, None));
    harness.run_steps(3);
    harness
        .get_all_by_label("Pop out")
        .next()
        .expect("a pop out button")
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().layout().windows.len(), 2);
}

#[test]
fn a_lone_window_has_no_pop_out_buttons() {
    let harness = harness();
    assert!(harness.query_by_label("Pop out").is_none());
}

#[test]
fn closing_the_window_tells_the_hub() {
    let bus = ScriptBus::default();
    let sent = Rc::clone(&bus.sent);
    let mut harness =
        Harness::new_eframe(|_| FusionApp::with_bus(Box::new(bus.clone()), WindowId(2), None));
    harness.run();
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.run();
    assert!(sent.borrow().contains(&ToHub::CloseWindow {
        window: WindowId(2)
    }));
}

#[test]
fn a_quit_from_the_hub_stops_drawing() {
    let bus = ScriptBus::default();
    bus.replies.borrow_mut().push(FromHub::Quit);
    let mut harness =
        Harness::new_eframe(|_| FusionApp::with_bus(Box::new(bus.clone()), WindowId(2), None));
    harness.run_steps(2);
    assert!(harness.state().quit_requested());
    assert!(harness.query_by_label("Panels").is_none());
}

#[test]
fn a_launched_window_leaves_the_layout_to_the_launcher() {
    let hub = SharedHub::new(default_layout());
    let mut app = FusionApp::with_bus(hub.bus(WindowId::MAIN), WindowId::MAIN, None);
    let mut storage = MemoryStorage::default();
    eframe::App::save(&mut app, &mut storage);
    assert_eq!(storage.get_string(LAYOUT_KEY), None);
    assert!(storage.get_string(THEME_KEY).is_some());
}
