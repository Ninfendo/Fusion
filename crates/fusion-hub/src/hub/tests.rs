use fusion_layout::{ContextType, Link, LinkColour, LinkMode};

use super::*;
use crate::local::{Bus, LocalBus};

const MAIN: WindowId = WindowId::MAIN;

/// Main window: [tabs: watchlist 1, L2 2, L2 3]. A link from 1 to 2.
fn layout() -> Layout {
    let mut layout = Layout::new("Fusion");
    for kind in ["watchlist", "l2", "l2"] {
        layout
            .add_panel(MAIN, Panel::new(kind, kind))
            .expect("added");
    }
    layout
        .add_link(link(1, &[2], LinkMode::All))
        .expect("valid");
    layout
}

fn link(from: u64, to: &[u64], mode: LinkMode) -> Link {
    Link {
        from: PanelId(from),
        context: ContextType::instrument(),
        to: to.iter().map(|id| PanelId(*id)).collect(),
        mode,
        gesture: None,
    }
}

fn eth() -> Fdc3Context {
    Fdc3Context::from(&Context::instrument("ETHUSDT"))
}

fn select(from: u64) -> ToHub {
    ToHub::Select {
        from: PanelId(from),
        context: eth(),
        gesture: None,
    }
}

fn deliveries(reply: &Reply) -> Vec<(Address, PanelId)> {
    reply
        .messages
        .iter()
        .filter_map(|out| match &out.message {
            FromHub::Deliver { panel, .. } => Some((out.to, *panel)),
            _ => None,
        })
        .collect()
}

fn notices(reply: &Reply) -> Vec<(Address, String)> {
    reply
        .messages
        .iter()
        .filter_map(|out| match &out.message {
            FromHub::Notice { text } => Some((out.to, text.clone())),
            _ => None,
        })
        .collect()
}

fn broadcast(reply: &Reply) -> bool {
    reply
        .messages
        .iter()
        .any(|out| out.to == Address::All && matches!(out.message, FromHub::Layout { .. }))
}

// --- Addresses ---------------------------------------------------------------

#[test]
fn addresses_reach_the_right_windows() {
    assert!(Address::All.reaches(WindowId(5)));
    assert!(Address::Window(WindowId(5)).reaches(WindowId(5)));
    assert!(!Address::Window(WindowId(5)).reaches(MAIN));
}

// --- Hello and restart ----------------------------------------------------------

#[test]
fn a_new_window_gets_the_layout() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(MAIN, ToHub::Hello { window: MAIN });
    assert_eq!(
        reply.messages,
        [Output {
            to: Address::Window(MAIN),
            message: FromHub::Layout {
                layout: hub.layout().clone()
            },
        }]
    );
    assert!(reply.opened.is_empty() && reply.closed.is_empty() && !reply.quit);
}

#[test]
fn a_window_that_is_not_in_the_layout_is_told_to_quit() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(
        WindowId(7),
        ToHub::Hello {
            window: WindowId(7),
        },
    );
    assert_eq!(
        reply.messages,
        [Output {
            to: Address::Window(WindowId(7)),
            message: FromHub::Quit,
        }]
    );
}

#[test]
fn a_selection_reaches_only_the_linked_panel_in_another_window() {
    let mut hub = Hub::new(layout());
    let popped = hub.handle(MAIN, ToHub::PopOut { panel: PanelId(2) });
    assert_eq!(popped.opened, [WindowId(2)]);
    let reply = hub.handle(MAIN, select(1));
    assert_eq!(
        deliveries(&reply),
        [(Address::Window(WindowId(2)), PanelId(2))]
    );
    assert!(!broadcast(&reply), "a selection does not change the layout");
}

#[test]
fn a_restarted_window_gets_its_last_contexts_back() {
    let mut hub = Hub::new(layout());
    hub.handle(MAIN, ToHub::PopOut { panel: PanelId(2) });
    hub.handle(MAIN, select(1));
    // The window process died and came back: it says hello again.
    let reply = hub.handle(
        WindowId(2),
        ToHub::Hello {
            window: WindowId(2),
        },
    );
    assert!(matches!(reply.messages[0].message, FromHub::Layout { .. }));
    assert_eq!(
        reply.messages[1],
        Output {
            to: Address::Window(WindowId(2)),
            message: FromHub::Deliver {
                panel: PanelId(2),
                context: eth(),
            },
        }
    );
}

#[test]
fn an_unlinked_selection_reaches_nobody() {
    let mut hub = Hub::new(layout());
    assert!(hub.handle(MAIN, select(3)).messages.is_empty());
}

#[test]
fn last_focused_links_follow_focus_messages() {
    let mut l = layout();
    l.add_link(link(1, &[2, 3], LinkMode::LastFocused))
        .expect("valid");
    l.remove_link(0).expect("the first link exists");
    let mut hub = Hub::new(l);
    hub.handle(MAIN, ToHub::Focused { panel: PanelId(3) });
    let reply = hub.handle(MAIN, select(1));
    assert_eq!(deliveries(&reply), [(Address::Window(MAIN), PanelId(3))]);
}

// --- Dock edits ---------------------------------------------------------------------

#[test]
fn a_dock_edit_updates_the_layout_and_tells_every_window() {
    let mut hub = Hub::new(layout());
    let root = Node::split(
        fusion_layout::Direction::Vertical,
        vec![
            Node::tabs(vec![Node::panel(PanelId(3)), Node::panel(PanelId(1))]),
            Node::tabs(vec![Node::panel(PanelId(2))]),
        ],
    );
    let reply = hub.handle(
        MAIN,
        ToHub::WindowEdited {
            window: MAIN,
            root: Some(root.clone()),
            closed: vec![],
        },
    );
    assert!(broadcast(&reply));
    assert_eq!(
        hub.layout().window(MAIN).and_then(|w| w.root.clone()),
        Some(root)
    );
}

#[test]
fn closed_tabs_leave_the_layout_and_the_router() {
    let mut hub = Hub::new(layout());
    hub.handle(MAIN, select(1));
    let reply = hub.handle(
        MAIN,
        ToHub::WindowEdited {
            window: MAIN,
            root: Some(Node::tabs(vec![
                Node::panel(PanelId(1)),
                Node::panel(PanelId(3)),
            ])),
            closed: vec![PanelId(2)],
        },
    );
    assert!(broadcast(&reply));
    assert!(hub.layout().panel(PanelId(2)).is_none());
    assert!(
        hub.layout().links.is_empty(),
        "the link to panel 2 went with it"
    );
    assert_eq!(hub.layout().validate(), Ok(()));
}

#[test]
fn a_dock_edit_that_loses_a_panel_is_refused_and_resynchronised() {
    let mut hub = Hub::new(layout());
    let before = hub.layout().clone();
    let reply = hub.handle(
        MAIN,
        ToHub::WindowEdited {
            window: MAIN,
            root: Some(Node::tabs(vec![Node::panel(PanelId(1))])),
            closed: vec![],
        },
    );
    assert_eq!(hub.layout(), &before);
    assert_eq!(
        reply.messages,
        [Output {
            to: Address::Window(MAIN),
            message: FromHub::Layout { layout: before },
        }]
    );
}

#[test]
fn a_broken_tree_is_refused_and_resynchronised() {
    let mut hub = Hub::new(layout());
    let bad = Node::Tabs {
        active: 9,
        children: vec![
            Node::panel(PanelId(1)),
            Node::panel(PanelId(2)),
            Node::panel(PanelId(3)),
        ],
    };
    let reply = hub.handle(
        MAIN,
        ToHub::WindowEdited {
            window: MAIN,
            root: Some(bad),
            closed: vec![],
        },
    );
    assert!(!broadcast(&reply));
    assert_eq!(reply.messages.len(), 1);
}

#[test]
fn a_close_alone_still_counts_when_the_tree_is_refused() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(
        MAIN,
        ToHub::WindowEdited {
            window: MAIN,
            root: None,
            closed: vec![PanelId(3)],
        },
    );
    assert!(hub.layout().panel(PanelId(3)).is_none());
    assert!(broadcast(&reply), "the close changed the layout");
}

// --- Panels, groups, links --------------------------------------------------------

#[test]
fn adding_a_panel_tells_every_window() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(
        MAIN,
        ToHub::AddPanel {
            window: MAIN,
            kind: "log".to_owned(),
            title: "Log".to_owned(),
        },
    );
    assert!(broadcast(&reply));
    assert_eq!(
        hub.layout().panel(PanelId(4)).map(|p| p.kind.as_str()),
        Some("log")
    );
}

#[test]
fn a_refused_change_tells_only_the_sender_why() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(
        WindowId(3),
        ToHub::AddPanel {
            window: WindowId(9),
            kind: "log".to_owned(),
            title: "Log".to_owned(),
        },
    );
    assert_eq!(
        notices(&reply),
        [(
            Address::Window(WindowId(3)),
            "Refused: there is no window 9".to_owned()
        )]
    );
    assert!(!broadcast(&reply));
}

#[test]
fn groups_are_set_through_the_hub() {
    let mut hub = Hub::new(layout());
    assert!(broadcast(&hub.handle(
        MAIN,
        ToHub::SetGroup {
            panel: PanelId(3),
            colour: Some(LinkColour::Red),
        }
    )));
    assert_eq!(hub.layout().groups.get(&PanelId(3)), Some(&LinkColour::Red));
}

#[test]
fn link_changes_restart_round_robin_turns() {
    let mut hub = Hub::new(layout());
    hub.handle(MAIN, ToHub::RemoveLink { index: 0 });
    hub.handle(
        MAIN,
        ToHub::AddLink {
            link: link(1, &[2, 3], LinkMode::RoundRobin),
        },
    );
    assert_eq!(deliveries(&hub.handle(MAIN, select(1)))[0].1, PanelId(2));
    hub.handle(
        MAIN,
        ToHub::AddLink {
            link: link(3, &[2], LinkMode::All),
        },
    );
    assert_eq!(
        deliveries(&hub.handle(MAIN, select(1)))[0].1,
        PanelId(2),
        "the turns started over"
    );
}

#[test]
fn removing_an_unknown_link_is_refused() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(MAIN, ToHub::RemoveLink { index: 5 });
    assert_eq!(
        notices(&reply),
        [(
            Address::Window(MAIN),
            "Refused: there is no link 5".to_owned()
        )]
    );
}

#[test]
fn a_new_layout_replaces_the_old_one_and_resets_the_router() {
    let mut hub = Hub::new(layout());
    hub.handle(MAIN, ToHub::PopOut { panel: PanelId(3) });
    hub.handle(MAIN, select(1));
    let fresh = layout();
    let reply = hub.handle(
        MAIN,
        ToHub::ReplaceLayout {
            layout: fresh.clone(),
        },
    );
    assert_eq!(hub.layout(), &fresh);
    assert_eq!(reply.closed, [WindowId(2)]);
    let back = hub.handle(WindowId(1), ToHub::Hello { window: MAIN });
    assert_eq!(back.messages.len(), 1, "no last contexts after a reset");
}

#[test]
fn a_broken_layout_is_not_taken() {
    let mut hub = Hub::new(layout());
    let mut broken = layout();
    broken.windows.clear();
    let reply = hub.handle(MAIN, ToHub::ReplaceLayout { layout: broken });
    assert_eq!(hub.layout(), &layout());
    assert_eq!(notices(&reply).len(), 1);
}

// --- Windows -----------------------------------------------------------------------

#[test]
fn pop_out_opens_a_window_and_pop_in_closes_it() {
    let mut hub = Hub::new(layout());
    let out = hub.handle(MAIN, ToHub::PopOut { panel: PanelId(3) });
    assert_eq!(
        (out.opened.as_slice(), out.closed.as_slice()),
        (&[WindowId(2)][..], &[][..])
    );
    assert!(broadcast(&out));
    let back = hub.handle(WindowId(2), ToHub::PopIn { panel: PanelId(3) });
    assert_eq!(
        (back.opened.as_slice(), back.closed.as_slice()),
        (&[][..], &[WindowId(2)][..])
    );
    assert_eq!(hub.window_ids(), [MAIN]);
}

#[test]
fn closing_a_window_pops_its_panels_in() {
    let mut hub = Hub::new(layout());
    hub.handle(MAIN, ToHub::PopOut { panel: PanelId(3) });
    let reply = hub.handle(
        WindowId(2),
        ToHub::CloseWindow {
            window: WindowId(2),
        },
    );
    assert_eq!(reply.closed, [WindowId(2)]);
    assert!(!reply.quit);
    assert_eq!(hub.layout().window_of(PanelId(3)), Some(MAIN));
}

#[test]
fn closing_the_main_window_ends_the_app() {
    let mut hub = Hub::new(layout());
    let reply = hub.handle(MAIN, ToHub::CloseWindow { window: MAIN });
    assert!(reply.quit);
    assert!(reply.messages.is_empty());
    assert_eq!(hub.layout(), &layout(), "the layout is kept for next time");
}

#[test]
fn bad_window_requests_are_refused() {
    let mut hub = Hub::new(layout());
    assert_eq!(
        notices(&hub.handle(MAIN, ToHub::PopIn { panel: PanelId(1) })).len(),
        1
    );
    assert_eq!(
        notices(&hub.handle(
            MAIN,
            ToHub::CloseWindow {
                window: WindowId(9)
            }
        ))
        .len(),
        1
    );
}

// --- The in-process bus -------------------------------------------------------------

#[test]
fn the_local_bus_starts_with_the_layout() {
    let mut bus = LocalBus::new(layout());
    assert!(!bus.can_open_windows());
    let first = bus.poll();
    assert_eq!(
        first,
        [FromHub::Layout {
            layout: bus.hub().layout().clone()
        }]
    );
    assert!(bus.poll().is_empty(), "poll takes the messages");
}

#[test]
fn the_local_bus_brings_other_windows_home() {
    let mut l = layout();
    l.pop_out(PanelId(2)).expect("popped out");
    let bus = LocalBus::new(l);
    assert_eq!(bus.hub().window_ids(), [MAIN]);
    assert_eq!(bus.hub().layout().panels_in(MAIN).len(), 3);
}

#[test]
fn the_local_bus_delivers_within_its_window() {
    let mut bus = LocalBus::new(layout());
    bus.poll();
    bus.send(select(1));
    assert_eq!(
        bus.poll(),
        [FromHub::Deliver {
            panel: PanelId(2),
            context: eth(),
        }]
    );
}
