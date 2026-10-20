use fusion_client_core::links::Context;
use fusion_layout::{ContextType, Link, LinkMode, Panel};

use super::*;
use crate::protocol::{Fdc3Context, decode, encode};

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
        .add_link(Link {
            from: PanelId(1),
            context: ContextType::instrument(),
            to: vec![PanelId(2)],
            mode: LinkMode::All,
            gesture: None,
        })
        .expect("valid");
    layout
}

fn eth() -> Fdc3Context {
    Fdc3Context::from(&Context::instrument("ETHUSDT"))
}

/// The posts for `window`, as that tab reads them.
fn read_by(posts: &[Envelope], window: WindowId) -> Vec<FromHub> {
    posts
        .iter()
        .cloned()
        .filter_map(|post| post.for_window(window))
        .collect()
}

#[test]
fn envelopes_travel_as_json() {
    let post = Envelope::ToHub {
        from: WindowId(2),
        message: ToHub::Focused { panel: PanelId(3) },
    };
    let line = encode(&post).expect("encodes");
    assert_eq!(decode::<Envelope>(&line).expect("decodes"), post);
    let started = encode(&Envelope::HubStarted).expect("encodes");
    assert_eq!(started, "\"hub-started\"\n");
}

#[test]
fn a_post_is_read_only_by_its_window() {
    let post = Envelope::FromHub {
        to: Address::Window(WindowId(2)),
        message: FromHub::Quit,
    };
    assert_eq!(post.clone().for_window(WindowId(2)), Some(FromHub::Quit));
    assert_eq!(post.for_window(MAIN), None);
    assert_eq!(Envelope::HubStarted.for_window(MAIN), None);
}

#[test]
fn leftover_windows_come_back_into_the_main_window() {
    let mut saved = layout();
    saved.pop_out(PanelId(2)).expect("pops out");
    let hub = TabHub::new(saved);
    assert_eq!(hub.layout().windows.len(), 1);
    assert_eq!(hub.layout().panels_in(MAIN).len(), 3);
}

#[test]
fn the_hub_tab_gets_its_own_messages_and_posts_the_rest() {
    let mut hub = TabHub::new(layout());
    let hello = hub.handle(MAIN, ToHub::Hello { window: MAIN });
    assert!(matches!(hello.mine.as_slice(), [FromHub::Layout { .. }]));
    assert!(!hello.changed, "a hello changes nothing");
    assert_eq!(
        read_by(&hello.post, MAIN).len(),
        1,
        "a second copy of the main tab hears it too"
    );
    assert!(read_by(&hello.post, WindowId(2)).is_empty());

    let popped = hub.handle(MAIN, ToHub::PopOut { panel: PanelId(2) });
    assert!(popped.changed);
    let window = hub.layout().window_of(PanelId(2)).expect("popped out");
    assert!(matches!(
        read_by(&popped.post, window).as_slice(),
        [FromHub::Layout { .. }]
    ));

    let selected = hub.handle(
        MAIN,
        ToHub::Select {
            from: PanelId(1),
            context: eth(),
            gesture: None,
        },
    );
    assert!(selected.mine.is_empty(), "panel 2 is not in the main tab");
    assert_eq!(
        read_by(&selected.post, window),
        [FromHub::Deliver {
            panel: PanelId(2),
            context: eth(),
        }]
    );
}

#[test]
fn closing_the_main_window_closes_every_tab() {
    let mut hub = TabHub::new(layout());
    let reply = hub.handle(MAIN, ToHub::CloseWindow { window: MAIN });
    assert_eq!(read_by(&reply.post, WindowId(5)), [FromHub::Quit]);
}

#[test]
fn a_popup_shows_once_the_hub_moved_its_panel() {
    let mut popups = PendingPopups::default();
    assert!(popups.is_empty());
    popups.add(PanelId(2), "popup", 10.0);
    assert!(!popups.is_empty());
    let before = layout();
    assert!(
        popups.settle(Some(&before), MAIN, 10.1).is_empty(),
        "an older layout: keep waiting"
    );
    assert!(popups.settle(None, MAIN, 10.2).is_empty());
    let mut after = layout();
    let window = after.pop_out(PanelId(2)).expect("pops out");
    assert_eq!(
        popups.settle(Some(&after), MAIN, 10.3),
        [Popup::Show("popup", window)]
    );
    assert!(popups.is_empty());
}

#[test]
fn a_popup_the_hub_never_fills_closes() {
    let mut popups = PendingPopups::default();
    popups.add(PanelId(2), "popup", 10.0);
    assert!(
        popups
            .settle(None, MAIN, 10.0 + POPUP_WAIT_SECONDS)
            .is_empty()
    );
    assert_eq!(
        popups.settle(Some(&layout()), MAIN, 10.1 + POPUP_WAIT_SECONDS),
        [Popup::Close("popup")]
    );
}

#[test]
fn the_page_address_names_the_window() {
    assert_eq!(window_from_query(""), MAIN);
    assert_eq!(window_from_query("?window=3"), WindowId(3));
    assert_eq!(window_from_query("?a=b&window=4"), WindowId(4));
    assert_eq!(window_from_query("window=5"), WindowId(5));
    assert_eq!(window_from_query("?window=x"), MAIN);
}
