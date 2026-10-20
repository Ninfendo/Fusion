use egui_kittest::{Harness, kittest::Queryable as _};

use super::*;

fn p(id: u64) -> Node {
    Node::panel(PanelId(id))
}

/// [tabs 1 2 (2 active)] | [ [tabs 3] / [tabs 4 5] ], with uneven shares.
fn sample() -> Node {
    Node::Split {
        direction: Direction::Horizontal,
        children: vec![
            Child {
                share: 3.0,
                node: Node::Tabs {
                    active: 1,
                    children: vec![p(1), p(2)],
                },
            },
            Child {
                share: 2.0,
                node: Node::Split {
                    direction: Direction::Vertical,
                    children: vec![
                        Child {
                            share: 1.0,
                            node: Node::tabs(vec![p(3)]),
                        },
                        Child {
                            share: 4.0,
                            node: Node::tabs(vec![p(4), p(5)]),
                        },
                    ],
                },
            },
        ],
    }
}

// --- Conversion ------------------------------------------------------------

#[test]
fn a_layout_survives_the_trip_through_egui_tiles() {
    let node = sample();
    let tree = to_tree(egui::Id::unique("dock"), Some(&node));
    assert_eq!(to_node(&tree), Some(node));
}

#[test]
fn an_empty_window_gives_an_empty_tree() {
    let tree = to_tree(egui::Id::unique("dock"), None);
    assert!(tree.is_empty());
    assert_eq!(to_node(&tree), None);
}

#[test]
fn a_lone_panel_survives() {
    let tree = to_tree(egui::Id::unique("dock"), Some(&p(7)));
    assert_eq!(to_node(&tree), Some(p(7)));
}

#[test]
fn the_active_tab_is_kept() {
    for active in 0..3 {
        let node = Node::Tabs {
            active,
            children: vec![p(1), p(2), p(3)],
        };
        assert_eq!(
            to_node(&to_tree(egui::Id::unique("dock"), Some(&node))),
            Some(node)
        );
    }
}

#[test]
fn an_empty_container_reads_as_nothing() {
    let mut tiles = Tiles::default();
    let tabs = tiles.insert_tab_tile(vec![]);
    let tree = Tree::new(egui::Id::unique("dock"), tabs, tiles);
    assert_eq!(to_node(&tree), None);

    let mut tiles: Tiles<PanelId> = Tiles::default();
    let split = tiles.insert_horizontal_tile(vec![]);
    let tree = Tree::new(egui::Id::unique("dock"), split, tiles);
    assert_eq!(to_node(&tree), None);
}

#[test]
fn a_tab_that_points_nowhere_is_skipped_and_the_active_tab_follows() {
    let mut tiles = Tiles::default();
    let one = tiles.insert_pane(PanelId(1));
    let two = tiles.insert_pane(PanelId(2));
    let tabs = tiles.insert_tab_tile(vec![TileId::from_u64(999), one, two]);
    if let Some(Tile::Container(Container::Tabs(t))) = tiles.get_mut(tabs) {
        t.set_active(two);
    }
    let tree = Tree::new(egui::Id::unique("dock"), tabs, tiles);
    assert_eq!(
        to_node(&tree),
        Some(Node::Tabs {
            active: 1,
            children: vec![p(1), p(2)],
        })
    );
}

#[test]
fn a_grid_reads_as_tabs() {
    let mut tiles = Tiles::default();
    let one = tiles.insert_pane(PanelId(1));
    let two = tiles.insert_pane(PanelId(2));
    let grid = tiles.insert_grid_tile(vec![one, two]);
    let tree = Tree::new(egui::Id::unique("dock"), grid, tiles);
    assert_eq!(to_node(&tree), Some(Node::tabs(vec![p(1), p(2)])));

    let mut tiles: Tiles<PanelId> = Tiles::default();
    let empty = tiles.insert_grid_tile(vec![]);
    assert_eq!(
        to_node(&Tree::new(egui::Id::unique("dock"), empty, tiles)),
        None
    );
}

// --- Behaviour -----------------------------------------------------------------

#[derive(Default)]
struct FakeHost {
    drawn: Vec<PanelId>,
}

impl PanelHost for FakeHost {
    fn title(&self, id: PanelId) -> egui::WidgetText {
        format!("Panel {}", id.0).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, id: PanelId) {
        self.drawn.push(id);
        ui.label(format!("Body of {}", id.0));
    }
}

#[test]
fn only_panel_tabs_can_be_closed() {
    let mut host = FakeHost::default();
    let behavior = DockBehavior {
        host: &mut host,
        output: DockOutput::default(),
    };
    let mut tiles = Tiles::default();
    let pane = tiles.insert_pane(PanelId(1));
    let tabs = tiles.insert_tab_tile(vec![pane]);
    assert!(behavior.is_tab_closable(&tiles, pane));
    assert!(!behavior.is_tab_closable(&tiles, tabs));
}

#[test]
fn closing_a_tab_reports_the_panel() {
    let mut host = FakeHost::default();
    let mut behavior = DockBehavior {
        host: &mut host,
        output: DockOutput::default(),
    };
    let mut tiles = Tiles::default();
    let pane = tiles.insert_pane(PanelId(4));
    let tabs = tiles.insert_tab_tile(vec![pane]);
    assert!(behavior.on_tab_close(&mut tiles, pane));
    assert!(behavior.on_tab_close(&mut tiles, tabs));
    assert_eq!(
        behavior.output,
        DockOutput {
            edited: true,
            closed: vec![PanelId(4)],
        }
    );
}

#[test]
fn any_edit_marks_the_dock_edited() {
    let mut host = FakeHost::default();
    let mut behavior = DockBehavior {
        host: &mut host,
        output: DockOutput::default(),
    };
    behavior.on_edit(EditAction::TabSelected);
    assert!(behavior.output.edited);
    assert!(behavior.output.closed.is_empty());
}

#[test]
fn every_panel_keeps_a_tab() {
    let mut host = FakeHost::default();
    let behavior = DockBehavior {
        host: &mut host,
        output: DockOutput::default(),
    };
    let options = behavior.simplification_options();
    assert!(options.all_panes_must_have_tabs);
    assert_eq!(
        SimplificationOptions {
            all_panes_must_have_tabs: false,
            ..options
        },
        SimplificationOptions::default()
    );
}

// --- Drawing ---------------------------------------------------------------------

#[test]
fn drawing_shows_every_tab_and_the_active_panels() {
    let mut tree = to_tree(egui::Id::unique("dock"), Some(&sample()));
    let mut host = FakeHost::default();
    let mut output = DockOutput::default();
    {
        let mut harness = Harness::new_ui(|ui| {
            output = show(&mut tree, ui, &mut host);
        });
        harness.run();
        for id in 1..=5 {
            assert!(
                harness.query_by_label(&format!("Panel {id}")).is_some(),
                "tab of panel {id}"
            );
        }
        // Active tabs only: 2 (not 1), 3 and 4 (not 5).
        assert!(harness.query_by_label("Body of 2").is_some());
        assert!(harness.query_by_label("Body of 1").is_none());
        assert!(harness.query_by_label("Body of 3").is_some());
        assert!(harness.query_by_label("Body of 4").is_some());
    }
    assert!(host.drawn.contains(&PanelId(2)));
    assert!(!host.drawn.contains(&PanelId(1)));
    assert!(output.closed.is_empty());
}

#[test]
fn clicking_a_tab_switches_and_reports_an_edit() {
    let mut tree = to_tree(egui::Id::unique("dock"), Some(&sample()));
    let mut host = FakeHost::default();
    let mut edited = false;
    {
        let mut harness = Harness::new_ui(|ui| {
            edited |= show(&mut tree, ui, &mut host).edited;
        });
        harness.run();
        harness.get_by_label("Panel 1").click();
        harness.run();
        assert!(harness.query_by_label("Body of 1").is_some());
    }
    assert!(edited);
    let Some(Node::Split { children, .. }) = to_node(&tree) else {
        panic!("still a split");
    };
    assert_eq!(
        children[0].node,
        Node::Tabs {
            active: 0,
            children: vec![p(1), p(2)],
        }
    );
}
