use std::collections::BTreeMap;

use eframe::Storage as _;
use egui_kittest::{Harness, kittest::Queryable as _};

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
    let mut host = Host {
        layout: &app.layout,
        views: &mut app.views,
        theme: &theme,
        market: &app.market,
        now: 0.0,
        events: &mut app.events,
    };
    assert_eq!(host.title(PanelId(9)), "Panel 9");
    let mut harness = Harness::new_ui(|ui| host.ui(ui, PanelId(1)));
    harness.run();
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
