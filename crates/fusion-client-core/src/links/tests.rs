use fusion_layout::{Link, LinkColour, Node, Panel};

use super::*;

/// Panels: 1 watchlist, 2 and 3 L2 ladders, 4 chart, 5 log.
fn layout() -> Layout {
    let mut layout = Layout::new("Test");
    for (id, kind) in [
        (1, "watchlist"),
        (2, "l2"),
        (3, "l2"),
        (4, "chart"),
        (5, "log"),
    ] {
        layout.panels.insert(PanelId(id), Panel::new(kind, kind));
    }
    layout.next_panel_id = PanelId(6);
    layout.windows[0].root = Some(Node::tabs(
        (1..=5).map(|id| Node::panel(PanelId(id))).collect(),
    ));
    layout
}

fn link(from: u64, to: &[u64], mode: LinkMode, gesture: Option<Gesture>) -> Link {
    Link {
        from: PanelId(from),
        context: ContextType::instrument(),
        to: to.iter().map(|id| PanelId(*id)).collect(),
        mode,
        gesture,
    }
}

fn ids(list: &[u64]) -> Vec<PanelId> {
    list.iter().map(|id| PanelId(*id)).collect()
}

fn eth() -> Context {
    Context::instrument("ETHUSDT")
}

#[test]
fn a_click_reaches_only_the_linked_l2() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2], LinkMode::All, None))
        .expect("valid");
    let mut router = LinkRouter::default();
    assert_eq!(router.route(&layout, PanelId(1), &eth(), None), ids(&[2]));
    assert_eq!(router.last_context(PanelId(2)), Some(&eth()));
    assert_eq!(router.last_context(PanelId(3)), None);
}

#[test]
fn an_unlinked_panel_reaches_nobody() {
    let layout = layout();
    let mut router = LinkRouter::default();
    assert!(router.route(&layout, PanelId(1), &eth(), None).is_empty());
}

#[test]
fn a_gesture_picks_its_own_link() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2, 4], LinkMode::All, None))
        .expect("valid");
    layout
        .add_link(link(1, &[3], LinkMode::All, Some(Gesture::Shift)))
        .expect("valid");
    let mut router = LinkRouter::default();
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), None),
        ids(&[2, 4])
    );
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), Some(Gesture::Shift)),
        ids(&[3])
    );
    assert!(
        router
            .route(&layout, PanelId(1), &eth(), Some(Gesture::Alt))
            .is_empty()
    );
}

#[test]
fn a_link_for_another_context_type_does_not_fire() {
    let mut layout = layout();
    layout
        .add_link(Link {
            context: ContextType("fdc3.order".to_owned()),
            ..link(1, &[2], LinkMode::All, None)
        })
        .expect("valid");
    let mut router = LinkRouter::default();
    assert!(router.route(&layout, PanelId(1), &eth(), None).is_empty());
}

#[test]
fn round_robin_takes_turns_and_wraps() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2, 3, 4], LinkMode::RoundRobin, None))
        .expect("valid");
    let mut router = LinkRouter::default();
    let picks: Vec<Vec<PanelId>> = (0..4)
        .map(|_| router.route(&layout, PanelId(1), &eth(), None))
        .collect();
    assert_eq!(picks, [ids(&[2]), ids(&[3]), ids(&[4]), ids(&[2])]);
}

#[test]
fn a_links_change_restarts_the_turns() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2, 3], LinkMode::RoundRobin, None))
        .expect("valid");
    let mut router = LinkRouter::default();
    router.route(&layout, PanelId(1), &eth(), None);
    router.links_changed();
    assert_eq!(router.route(&layout, PanelId(1), &eth(), None), ids(&[2]));
}

#[test]
fn last_focused_follows_the_last_click() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2, 3], LinkMode::LastFocused, None))
        .expect("valid");
    let mut router = LinkRouter::default();
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), None),
        ids(&[2]),
        "with no click yet, the first target"
    );
    router.focused(PanelId(3));
    router.focused(PanelId(5));
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), None),
        ids(&[3]),
        "panel 5 is not a target, so the last clicked target wins"
    );
    router.focused(PanelId(2));
    assert_eq!(router.route(&layout, PanelId(1), &eth(), None), ids(&[2]));
}

#[test]
fn focusing_again_moves_a_panel_to_the_end() {
    let mut router = LinkRouter::default();
    for id in [2, 3, 2] {
        router.focused(PanelId(id));
    }
    assert_eq!(router.focus, ids(&[3, 2]));
}

#[test]
fn a_colour_group_follows_on_a_plain_click_only() {
    let mut layout = layout();
    for id in [1, 2, 4] {
        layout
            .set_group(PanelId(id), Some(LinkColour::Red))
            .expect("known");
    }
    layout
        .set_group(PanelId(3), Some(LinkColour::Blue))
        .expect("known");
    let mut router = LinkRouter::default();
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), None),
        ids(&[2, 4])
    );
    assert_eq!(
        router.route(&layout, PanelId(4), &eth(), None),
        ids(&[1, 2])
    );
    assert!(
        router
            .route(&layout, PanelId(1), &eth(), Some(Gesture::Shift))
            .is_empty()
    );
    assert!(
        router.route(&layout, PanelId(5), &eth(), None).is_empty(),
        "panel 5 is in no group"
    );
}

#[test]
fn a_panel_reached_twice_appears_once() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2], LinkMode::All, None))
        .expect("valid");
    layout
        .add_link(link(1, &[2, 3], LinkMode::All, None))
        .expect("valid");
    layout
        .set_group(PanelId(1), Some(LinkColour::Green))
        .expect("known");
    layout
        .set_group(PanelId(3), Some(LinkColour::Green))
        .expect("known");
    let mut router = LinkRouter::default();
    assert_eq!(
        router.route(&layout, PanelId(1), &eth(), None),
        ids(&[2, 3])
    );
}

#[test]
fn forgetting_a_panel_drops_its_state() {
    let mut layout = layout();
    layout
        .add_link(link(1, &[2], LinkMode::All, None))
        .expect("valid");
    let mut router = LinkRouter::default();
    router.focused(PanelId(2));
    router.route(&layout, PanelId(1), &eth(), None);
    router.forget(PanelId(2));
    assert_eq!(router.last_context(PanelId(2)), None);
    assert!(router.focus.is_empty());
}

#[test]
fn contexts_know_their_fdc3_type() {
    assert_eq!(eth().context_type(), ContextType::instrument());
    assert_eq!(
        eth(),
        Context::Instrument {
            ticker: "ETHUSDT".to_owned()
        }
    );
}
