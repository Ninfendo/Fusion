use super::*;

fn watchlist() -> Panel {
    Panel::new("watchlist", "Watchlist")
}

/// Main window: a horizontal split of [tabs: 1, 2] and [tabs: 3].
fn sample() -> Layout {
    let mut layout = Layout::new("Fusion");
    for (id, kind) in [(1, "watchlist"), (2, "log"), (3, "l2")] {
        layout.panels.insert(PanelId(id), Panel::new(kind, kind));
    }
    layout.next_panel_id = PanelId(4);
    layout.windows[0].root = Some(Node::Split {
        direction: Direction::Horizontal,
        children: vec![
            Child {
                share: 2.0,
                node: Node::tabs(vec![Node::panel(PanelId(1)), Node::panel(PanelId(2))]),
            },
            Child {
                share: 1.0,
                node: Node::tabs(vec![Node::panel(PanelId(3))]),
            },
        ],
    });
    layout
}

// --- Building ---------------------------------------------------------------

#[test]
fn a_new_layout_has_one_empty_main_window() {
    let layout = Layout::new("Fusion");
    assert_eq!(layout.windows.len(), 1);
    assert_eq!(layout.windows[0].id, WindowId::MAIN);
    assert_eq!(layout.windows[0].root, None);
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn the_first_panel_gets_a_tab_group() {
    let mut layout = Layout::new("Fusion");
    let id = layout
        .add_panel(WindowId::MAIN, watchlist())
        .expect("added");
    assert_eq!(id, PanelId(1));
    assert_eq!(
        layout.windows[0].root,
        Some(Node::tabs(vec![Node::panel(PanelId(1))]))
    );
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn more_panels_join_the_first_tab_group() {
    let mut layout = sample();
    let id = layout
        .add_panel(WindowId::MAIN, watchlist())
        .expect("added");
    assert_eq!(id, PanelId(4));
    assert_eq!(
        layout.panels_in(WindowId::MAIN),
        [PanelId(1), PanelId(2), PanelId(4), PanelId(3)]
    );
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn a_tree_without_tabs_gets_a_new_tab_group_beside_it() {
    let mut layout = Layout::new("Fusion");
    layout.panels.insert(PanelId(1), watchlist());
    layout.next_panel_id = PanelId(2);
    layout.windows[0].root = Some(Node::panel(PanelId(1)));
    layout
        .add_panel(WindowId::MAIN, watchlist())
        .expect("added");
    assert_eq!(
        layout.windows[0].root,
        Some(Node::split(
            Direction::Horizontal,
            vec![
                Node::panel(PanelId(1)),
                Node::tabs(vec![Node::panel(PanelId(2))])
            ]
        ))
    );
}

#[test]
fn adding_to_an_unknown_window_is_refused() {
    let mut layout = Layout::new("Fusion");
    assert_eq!(
        layout.add_panel(WindowId(9), watchlist()),
        Err(LayoutError::UnknownWindow(WindowId(9)))
    );
    assert!(layout.panels.is_empty());
}

#[test]
fn panel_ids_are_never_reused() {
    let mut layout = sample();
    layout.close_panel(PanelId(3)).expect("closed");
    let id = layout
        .add_panel(WindowId::MAIN, watchlist())
        .expect("added");
    assert_eq!(id, PanelId(4), "the closed panel's id 3 is not reused");
    assert_eq!(layout.next_panel_id, PanelId(5));
}

#[test]
fn lookups_find_windows_and_panels() {
    let layout = sample();
    assert_eq!(
        layout.window(WindowId::MAIN).map(|w| w.title.as_str()),
        Some("Fusion")
    );
    assert!(layout.window(WindowId(2)).is_none());
    assert_eq!(
        layout.panel(PanelId(3)).map(|p| p.kind.as_str()),
        Some("l2")
    );
    assert!(layout.panel(PanelId(9)).is_none());
    assert!(layout.panels_in(WindowId(2)).is_empty());
}

// --- Closing ------------------------------------------------------------------

#[test]
fn closing_a_panel_removes_it_everywhere() {
    let mut layout = sample();
    let closed = layout.close_panel(PanelId(2)).expect("closed");
    assert_eq!(closed.kind, "log");
    assert_eq!(layout.panels_in(WindowId::MAIN), [PanelId(1), PanelId(3)]);
    assert!(layout.panel(PanelId(2)).is_none());
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn closing_the_last_panel_of_a_split_side_collapses_the_split() {
    let mut layout = sample();
    layout.close_panel(PanelId(3)).expect("closed");
    assert_eq!(
        layout.windows[0].root,
        Some(Node::tabs(vec![
            Node::panel(PanelId(1)),
            Node::panel(PanelId(2))
        ]))
    );
}

#[test]
fn closing_every_panel_empties_the_window() {
    let mut layout = sample();
    for id in [1, 2, 3] {
        layout.close_panel(PanelId(id)).expect("closed");
    }
    assert_eq!(layout.windows[0].root, None);
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn closing_an_unknown_panel_is_refused() {
    let mut layout = sample();
    assert_eq!(
        layout.close_panel(PanelId(9)),
        Err(LayoutError::UnknownPanel(PanelId(9)))
    );
}

#[test]
fn closing_a_tab_left_of_the_active_one_keeps_it_showing() {
    let mut layout = sample();
    if let Some(Node::Split { children, .. }) = &mut layout.windows[0].root
        && let Node::Tabs { active, .. } = &mut children[0].node
    {
        *active = 1;
    }
    layout.close_panel(PanelId(1)).expect("closed");
    let Some(Node::Split { children, .. }) = &layout.windows[0].root else {
        panic!("still a split");
    };
    assert_eq!(children[0].node, Node::tabs(vec![Node::panel(PanelId(2))]));
}

#[test]
fn closing_the_active_last_tab_shows_the_new_last_tab() {
    let tabs = Node::Tabs {
        active: 1,
        children: vec![Node::panel(PanelId(1)), Node::panel(PanelId(2))],
    };
    assert_eq!(
        tabs.without_panel(PanelId(2)).simplified(),
        Some(Node::tabs(vec![Node::panel(PanelId(1))]))
    );
}

#[test]
fn simplify_keeps_shares_and_drops_empty_children() {
    let tree = Node::Split {
        direction: Direction::Vertical,
        children: vec![
            Child {
                share: 3.0,
                node: Node::panel(PanelId(1)),
            },
            Child {
                share: 1.0,
                node: Node::tabs(vec![]),
            },
            Child {
                share: 2.0,
                node: Node::panel(PanelId(2)),
            },
        ],
    };
    assert_eq!(
        tree.simplified(),
        Some(Node::Split {
            direction: Direction::Vertical,
            children: vec![
                Child {
                    share: 3.0,
                    node: Node::panel(PanelId(1)),
                },
                Child {
                    share: 2.0,
                    node: Node::panel(PanelId(2)),
                },
            ],
        })
    );
}

#[test]
fn a_single_tab_group_stays_so_its_tab_can_be_dragged() {
    let tabs = Node::tabs(vec![Node::panel(PanelId(1))]);
    assert_eq!(tabs.clone().simplified(), Some(tabs));
}

// --- Validation ----------------------------------------------------------------

/// A change that breaks the sample layout, and the error it must cause.
type Breakage = (Box<dyn Fn(&mut Layout)>, LayoutError);

fn assert_refused(cases: Vec<Breakage>) {
    for (break_it, expected) in cases {
        let mut layout = sample();
        break_it(&mut layout);
        assert_eq!(layout.validate(), Err(expected.clone()), "{expected}");
    }
}

#[test]
fn validation_catches_broken_layouts() {
    assert_refused(vec![
        (
            Box::new(|l| l.schema_version = 1),
            LayoutError::UnsupportedVersion {
                found: 1,
                supported: 2,
            },
        ),
        (Box::new(|l| l.windows.clear()), LayoutError::NoWindows),
        (
            Box::new(|l| {
                let copy = l.windows[0].clone();
                l.windows.push(Window { root: None, ..copy });
            }),
            LayoutError::DuplicateWindow(WindowId::MAIN),
        ),
        (
            Box::new(|l| {
                l.panels.remove(&PanelId(3));
            }),
            LayoutError::UnknownPanel(PanelId(3)),
        ),
        (
            Box::new(|l| {
                l.panels.insert(PanelId(7), Panel::new("log", "Log"));
                l.next_panel_id = PanelId(8);
            }),
            LayoutError::PanelNotPlaced(PanelId(7)),
        ),
        (
            Box::new(|l| {
                l.windows.push(Window {
                    id: WindowId(2),
                    title: "Second".to_owned(),
                    bounds: None,
                    root: Some(Node::panel(PanelId(1))),
                });
            }),
            LayoutError::PanelPlacedTwice(PanelId(1)),
        ),
        (
            Box::new(|l| l.next_panel_id = PanelId(3)),
            LayoutError::NextIdTooLow {
                next: PanelId(3),
                highest: PanelId(3),
            },
        ),
    ]);
}

#[test]
fn validation_catches_broken_trees() {
    assert_refused(vec![
        (
            Box::new(|l| l.windows[0].root = Some(Node::tabs(vec![]))),
            LayoutError::EmptyContainer,
        ),
        (
            Box::new(|l| {
                l.windows[0].root = Some(Node::Split {
                    direction: Direction::Horizontal,
                    children: vec![],
                });
            }),
            LayoutError::EmptyContainer,
        ),
        (
            Box::new(|l| {
                l.windows[0].root = Some(Node::Tabs {
                    active: 3,
                    children: vec![
                        Node::panel(PanelId(1)),
                        Node::panel(PanelId(2)),
                        Node::panel(PanelId(3)),
                    ],
                });
            }),
            LayoutError::ActiveTabOutOfRange { active: 3, tabs: 3 },
        ),
        (
            Box::new(|l| {
                if let Some(Node::Split { children, .. }) = &mut l.windows[0].root {
                    children[1].share = 0.0;
                }
            }),
            LayoutError::BadShare(0.0),
        ),
    ]);
}

#[test]
fn a_share_that_is_not_a_number_is_refused() {
    let mut layout = sample();
    if let Some(Node::Split { children, .. }) = &mut layout.windows[0].root {
        children[0].share = f32::NAN;
    }
    assert!(matches!(layout.validate(), Err(LayoutError::BadShare(_))));
}

// --- JSON ---------------------------------------------------------------------------

#[test]
fn json_round_trip_keeps_everything() {
    let mut layout = sample();
    layout.windows[0].bounds = Some(Bounds {
        x: 10.0,
        y: 20.0,
        width: 1600.0,
        height: 900.0,
    });
    if let Some(panel) = layout.panels.get_mut(&PanelId(1)) {
        panel.config = serde_json::json!({ "columns": ["symbol", "bid", "ask"] });
    }
    let text = layout.to_json().expect("serialises");
    assert_eq!(Layout::from_json(&text), Ok(layout));
}

#[test]
fn the_json_is_readable() {
    let text = sample().to_json().expect("serialises");
    assert!(text.contains("\"schema_version\": 2"));
    assert!(text.contains("\"type\": \"split\""));
    assert!(text.contains("\"direction\": \"horizontal\""));
    assert!(text.contains("\"kind\": \"watchlist\""));
    assert!(!text.contains("\"config\""), "empty config is left out");
    assert!(!text.contains("\"bounds\""), "unknown bounds are left out");
    assert!(!text.contains("\"links\""), "no links, no field");
    assert!(!text.contains("\"groups\""), "no groups, no field");
}

#[test]
fn an_unknown_panel_kind_survives_a_round_trip() {
    let mut layout = sample();
    if let Some(panel) = layout.panels.get_mut(&PanelId(2)) {
        panel.kind = "from-a-newer-build".to_owned();
    }
    let text = layout.to_json().expect("serialises");
    let back = Layout::from_json(&text).expect("loads");
    assert_eq!(
        back.panel(PanelId(2)).map(|p| p.kind.as_str()),
        Some("from-a-newer-build")
    );
}

#[test]
fn a_newer_format_is_refused_before_parsing_the_rest() {
    let text = r#"{ "schema_version": 3, "something": "new" }"#;
    assert_eq!(
        Layout::from_json(text),
        Err(LayoutError::UnsupportedVersion {
            found: 3,
            supported: 2
        })
    );
    assert_eq!(
        Layout::from_json(r#"{ "schema_version": 0 }"#),
        Err(LayoutError::UnsupportedVersion {
            found: 0,
            supported: 2
        })
    );
}

#[test]
fn a_format_1_file_is_upgraded_on_load() {
    let text = sample()
        .to_json()
        .expect("serialises")
        .replace("\"schema_version\": 2", "\"schema_version\": 1");
    let loaded = Layout::from_json(&text).expect("format 1 still loads");
    assert_eq!(loaded.schema_version, 2);
    assert_eq!(loaded, sample());
}

#[test]
fn broken_json_is_refused() {
    assert!(matches!(
        Layout::from_json("{ not json"),
        Err(LayoutError::Json(_))
    ));
    assert!(matches!(
        Layout::from_json(r#"{ "schema_version": 1 }"#),
        Err(LayoutError::Json(_))
    ));
}

#[test]
fn a_loaded_layout_is_validated() {
    let mut layout = sample();
    layout.panels.insert(PanelId(7), Panel::new("log", "Log"));
    layout.next_panel_id = PanelId(8);
    let text = layout.to_json().expect("serialises");
    assert_eq!(
        Layout::from_json(&text),
        Err(LayoutError::PanelNotPlaced(PanelId(7)))
    );
}

#[test]
fn errors_read_as_sentences() {
    let cases = [
        (
            LayoutError::UnsupportedVersion {
                found: 3,
                supported: 2,
            },
            "layout format 3 is not supported; this build reads format 2",
        ),
        (
            LayoutError::Json("eof".to_owned()),
            "the layout file is not valid: eof",
        ),
        (LayoutError::NoWindows, "a layout needs at least one window"),
        (
            LayoutError::DuplicateWindow(WindowId(1)),
            "window 1 appears twice",
        ),
        (
            LayoutError::UnknownWindow(WindowId(2)),
            "there is no window 2",
        ),
        (LayoutError::UnknownPanel(PanelId(3)), "there is no panel 3"),
        (
            LayoutError::PanelPlacedTwice(PanelId(4)),
            "panel 4 is placed twice",
        ),
        (
            LayoutError::PanelNotPlaced(PanelId(5)),
            "panel 5 is in no window",
        ),
        (
            LayoutError::EmptyContainer,
            "a tab group or split has no children",
        ),
        (
            LayoutError::ActiveTabOutOfRange { active: 3, tabs: 2 },
            "active tab 3 is out of range for 2 tabs",
        ),
        (
            LayoutError::BadShare(0.0),
            "split share 0 must be above zero",
        ),
        (
            LayoutError::NextIdTooLow {
                next: PanelId(2),
                highest: PanelId(3),
            },
            "the next panel id 2 must be above the highest id in use, 3",
        ),
        (
            LayoutError::LinkWithoutTargets(PanelId(1)),
            "a link from panel 1 has no targets",
        ),
        (
            LayoutError::LinkToItself(PanelId(2)),
            "panel 2 cannot link to itself",
        ),
        (
            LayoutError::LinkTargetTwice(PanelId(3)),
            "panel 3 is a link target twice",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
    }
}

#[test]
fn closing_a_tab_left_of_the_active_one_keeps_the_same_tab_with_three_tabs() {
    let tabs = Node::Tabs {
        active: 1,
        children: vec![
            Node::panel(PanelId(1)),
            Node::panel(PanelId(2)),
            Node::panel(PanelId(3)),
        ],
    };
    assert_eq!(
        tabs.without_panel(PanelId(1)).simplified(),
        Some(Node::Tabs {
            active: 0,
            children: vec![Node::panel(PanelId(2)), Node::panel(PanelId(3))],
        }),
        "panel 2 was showing and must still show"
    );
}

#[test]
fn a_split_whose_children_all_vanish_vanishes() {
    let split = Node::split(
        Direction::Horizontal,
        vec![Node::tabs(vec![]), Node::tabs(vec![])],
    );
    assert_eq!(split.simplified(), None);
}

// --- Links and groups ------------------------------------------------------------

fn link(from: u64, to: &[u64]) -> Link {
    Link {
        from: PanelId(from),
        context: ContextType::instrument(),
        to: to.iter().map(|id| PanelId(*id)).collect(),
        mode: LinkMode::All,
        gesture: None,
    }
}

#[test]
fn a_valid_link_is_added() {
    let mut layout = sample();
    assert_eq!(layout.add_link(link(1, &[3])), Ok(()));
    assert_eq!(layout.links, [link(1, &[3])]);
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn a_bad_link_is_refused_and_not_kept() {
    let cases = [
        (link(9, &[3]), LayoutError::UnknownPanel(PanelId(9))),
        (link(1, &[]), LayoutError::LinkWithoutTargets(PanelId(1))),
        (link(1, &[9]), LayoutError::UnknownPanel(PanelId(9))),
        (link(1, &[1]), LayoutError::LinkToItself(PanelId(1))),
        (link(1, &[3, 3]), LayoutError::LinkTargetTwice(PanelId(3))),
    ];
    for (bad, expected) in cases {
        let mut layout = sample();
        assert_eq!(layout.add_link(bad), Err(expected));
        assert!(layout.links.is_empty());
    }
}

#[test]
fn links_are_removed_by_index() {
    let mut layout = sample();
    layout.add_link(link(1, &[3])).expect("valid");
    layout.add_link(link(2, &[3])).expect("valid");
    assert_eq!(layout.remove_link(0), Some(link(1, &[3])));
    assert_eq!(layout.links, [link(2, &[3])]);
    assert_eq!(layout.remove_link(1), None, "one past the end");
    assert_eq!(layout.remove_link(5), None);
}

#[test]
fn panels_join_and_leave_colour_groups() {
    let mut layout = sample();
    assert_eq!(layout.set_group(PanelId(1), Some(LinkColour::Red)), Ok(()));
    assert_eq!(layout.groups.get(&PanelId(1)), Some(&LinkColour::Red));
    assert_eq!(layout.set_group(PanelId(1), None), Ok(()));
    assert!(layout.groups.is_empty());
    assert_eq!(
        layout.set_group(PanelId(9), Some(LinkColour::Blue)),
        Err(LayoutError::UnknownPanel(PanelId(9)))
    );
}

#[test]
fn closing_a_panel_cleans_up_its_links_and_group() {
    let mut layout = sample();
    layout.add_link(link(1, &[2, 3])).expect("valid");
    layout.add_link(link(2, &[1])).expect("valid");
    layout.add_link(link(3, &[2])).expect("valid");
    layout
        .set_group(PanelId(2), Some(LinkColour::Green))
        .expect("known");
    layout.close_panel(PanelId(2)).expect("closed");
    // The link from 1 keeps its other target; the links from and to only 2 go.
    assert_eq!(layout.links, [link(1, &[3])]);
    assert!(layout.groups.is_empty());
    assert_eq!(layout.validate(), Ok(()));
}

#[test]
fn a_group_for_an_unknown_panel_is_refused() {
    let mut layout = sample();
    layout.groups.insert(PanelId(9), LinkColour::Red);
    assert_eq!(
        layout.validate(),
        Err(LayoutError::UnknownPanel(PanelId(9)))
    );
}

#[test]
fn links_and_groups_survive_json() {
    let mut layout = sample();
    layout
        .add_link(Link {
            mode: LinkMode::RoundRobin,
            gesture: Some(Gesture::Shift),
            ..link(1, &[2, 3])
        })
        .expect("valid");
    layout
        .set_group(PanelId(3), Some(LinkColour::Yellow))
        .expect("known");
    let text = layout.to_json().expect("serialises");
    assert!(text.contains("\"mode\": \"round-robin\""));
    assert!(text.contains("\"gesture\": \"shift\""));
    assert!(text.contains("\"context\": \"fdc3.instrument\""));
    assert!(text.contains("\"3\": \"yellow\""));
    assert_eq!(Layout::from_json(&text), Ok(layout));
}

#[test]
fn colours_have_names() {
    let names: Vec<&str> = LinkColour::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(names, ["Red", "Green", "Blue", "Yellow"]);
}
