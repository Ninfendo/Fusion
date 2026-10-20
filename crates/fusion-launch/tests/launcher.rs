//! The launcher with real pipes and pretend window processes: each test
//! connects windows of its own and drives the launcher turn by turn.

use std::cell::RefCell;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fusion_client_core::workspace::default_layout;
use fusion_hub::{Bus, Ended, Fdc3Context, FromHub, InstrumentIds, ToHub};
use fusion_launch::{Launcher, LayoutFile, WindowProcesses};
use fusion_layout::{Layout, LinkColour, PanelId, WindowId};
use fusion_pipe::{HubServer, PipeBus};
use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};

/// Real messages arrive in milliseconds; a short wait keeps a broken build
/// failing fast.
const WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct FakeState {
    started: Vec<WindowId>,
    ended: Vec<(WindowId, Ended)>,
    fail: bool,
    stopped: bool,
}

/// Pretend window processes: they record what the launcher asks.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<FakeState>>);

impl WindowProcesses for Fake {
    fn start(&mut self, window: WindowId) -> io::Result<()> {
        let mut state = self.0.borrow_mut();
        if state.fail {
            return Err(io::Error::other("no such program"));
        }
        state.started.push(window);
        Ok(())
    }

    fn ended(&mut self) -> Vec<(WindowId, Ended)> {
        std::mem::take(&mut self.0.borrow_mut().ended)
    }

    fn stop_all(&mut self, _grace: Duration) -> Vec<WindowId> {
        self.0.borrow_mut().stopped = true;
        vec![WindowId(7)]
    }
}

struct Rig {
    launcher: Launcher<Fake>,
    fake: Fake,
    name: String,
    file: PathBuf,
    log: Vec<String>,
}

impl Rig {
    fn new(test: &str, layout: Layout) -> Self {
        let tag = format!("fusion-launch-test-{}-{test}", std::process::id());
        let name = format!("{tag}.sock");
        let file = std::env::temp_dir().join(&tag).join("layout.json");
        let fake = Fake::default();
        let server = HubServer::listen(&name).expect("listens");
        let mut launcher = Launcher::new(layout, server, fake.clone(), LayoutFile::new(&file));
        launcher.start();
        Self {
            launcher,
            fake,
            name,
            file,
            log: Vec::new(),
        }
    }

    fn window(&self, window: WindowId) -> PipeBus {
        PipeBus::connect(&self.name, window, || {}).expect("connects")
    }

    fn turn(&mut self) -> bool {
        let running = self.launcher.turn(Duration::from_millis(5));
        self.log.extend(self.launcher.take_log());
        running
    }

    /// Turns the launcher until `bus` gets a message `want` accepts.
    /// Returns every message the bus got.
    fn until(&mut self, bus: &mut PipeBus, want: impl Fn(&FromHub) -> bool) -> Vec<FromHub> {
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < WAIT {
            self.turn();
            seen.extend(bus.poll());
            if seen.iter().any(&want) {
                return seen;
            }
        }
        panic!("waited in vain; the window saw {seen:?}");
    }

    /// Turns the launcher until `done` holds.
    fn until_rig(&mut self, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(start.elapsed() < WAIT, "waited in vain; log {:?}", self.log);
            self.turn();
        }
    }

    fn started(&self) -> Vec<WindowId> {
        self.fake.0.borrow().started.clone()
    }

    fn end(&self, window: WindowId, how: Ended) {
        self.fake.0.borrow_mut().ended.push((window, how));
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(folder) = self.file.parent() {
            // The folder is this test's own; a leftover is harmless.
            let _removed = std::fs::remove_dir_all(folder);
        }
    }
}

fn is_layout(message: &FromHub) -> bool {
    matches!(message, FromHub::Layout { .. })
}

/// The default layout with the L2 ladder (panel 2) in a window of its own.
fn popped_layout() -> (Layout, WindowId) {
    let mut layout = default_layout();
    let window = layout.pop_out(PanelId(2)).expect("pops out");
    (layout, window)
}

fn eth() -> Fdc3Context {
    Fdc3Context::Instrument {
        id: InstrumentIds {
            ticker: "ETHUSDT".to_owned(),
        },
    }
}

#[test]
fn every_window_gets_a_process() {
    let (layout, window) = popped_layout();
    let rig = Rig::new("start", layout);
    assert_eq!(rig.started(), [WindowId::MAIN, window]);
    assert!(!rig.launcher.hub().layout().windows.is_empty());
    assert!(rig.launcher.processes().0.borrow().ended.is_empty());
}

#[test]
fn a_selection_reaches_the_linked_panel_in_another_window() {
    let (layout, window) = popped_layout();
    let mut rig = Rig::new("gate", layout);
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    let mut other = rig.window(window);
    rig.until(&mut other, is_layout);

    main.send(ToHub::Select {
        from: PanelId(1),
        context: eth(),
        gesture: None,
    });
    let delivered = FromHub::Deliver {
        panel: PanelId(2),
        context: eth(),
    };
    rig.until(&mut other, |m| *m == delivered);
    assert!(
        !main
            .poll()
            .iter()
            .any(|m| matches!(m, FromHub::Deliver { .. })),
        "nothing in the main window is linked"
    );

    // The other window's process starts again and connects anew.
    let mut again = rig.window(window);
    rig.until(&mut again, |m| *m == delivered);
}

#[test]
fn popping_out_starts_a_window() {
    let mut rig = Rig::new("popout", default_layout());
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    main.send(ToHub::PopOut { panel: PanelId(3) });
    rig.until(
        &mut main,
        |m| matches!(m, FromHub::Layout { layout } if layout.windows.len() == 2),
    );
    assert_eq!(rig.started(), [WindowId::MAIN, WindowId(2)]);
}

#[test]
fn a_crashed_window_starts_again() {
    let (layout, window) = popped_layout();
    let mut rig = Rig::new("crash", layout);
    rig.end(window, Ended::Crashed);
    rig.turn();
    assert_eq!(rig.started(), [WindowId::MAIN, window, window]);
    assert!(rig.log.contains(&format!("Window {window} crashed")));
}

#[test]
fn a_closed_window_pops_its_panels_in() {
    let (layout, window) = popped_layout();
    let mut rig = Rig::new("closed", layout);
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    rig.end(window, Ended::Clean);
    rig.until(
        &mut main,
        |m| matches!(m, FromHub::Layout { layout } if layout.windows.len() == 1),
    );
    assert!(
        rig.launcher
            .hub()
            .layout()
            .panels_in(WindowId::MAIN)
            .contains(&PanelId(2))
    );
}

#[test]
fn a_window_that_cannot_start_pops_its_panels_in() {
    let mut rig = Rig::new("nostart", default_layout());
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    rig.fake.0.borrow_mut().fail = true;
    main.send(ToHub::PopOut { panel: PanelId(3) });
    rig.until_rig(|rig| !rig.log.is_empty());
    assert_eq!(
        rig.log,
        ["Window 2 could not start: no such program".to_owned()]
    );
    let layout = rig.launcher.hub().layout();
    assert_eq!(layout.windows.len(), 1);
    assert!(layout.panels_in(WindowId::MAIN).contains(&PanelId(3)));
}

#[test]
fn a_main_window_that_cannot_start_ends_the_app() {
    let file = std::env::temp_dir().join("fusion-launch-never-written.json");
    let fake = Fake::default();
    fake.0.borrow_mut().fail = true;
    let name = format!("fusion-launch-test-{}-nomain.sock", std::process::id());
    let server = HubServer::listen(&name).expect("listens");
    let mut launcher = Launcher::new(default_layout(), server, fake, LayoutFile::new(file));
    launcher.start();
    assert!(!launcher.turn(Duration::ZERO));
}

#[test]
fn a_changed_layout_is_saved_a_moment_later() {
    let mut rig = Rig::new("save", default_layout());
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    main.send(ToHub::SetGroup {
        panel: PanelId(1),
        colour: Some(LinkColour::Red),
    });
    rig.until(
        &mut main,
        |m| matches!(m, FromHub::Layout { layout } if !layout.groups.is_empty()),
    );
    assert!(!rig.file.exists(), "not at once");
    rig.until_rig(|rig| rig.file.exists());
    let saved = LayoutFile::new(&rig.file)
        .load()
        .expect("loads")
        .expect("saved");
    assert_eq!(saved.groups.get(&PanelId(1)), Some(&LinkColour::Red));
}

#[test]
fn closing_the_main_window_ends_the_app_and_saves() {
    let mut rig = Rig::new("quit", default_layout());
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    main.send(ToHub::PopOut { panel: PanelId(3) });
    rig.until(
        &mut main,
        |m| matches!(m, FromHub::Layout { layout } if layout.windows.len() == 2),
    );
    main.send(ToHub::CloseWindow {
        window: WindowId::MAIN,
    });
    let start = Instant::now();
    while rig.turn() {
        assert!(start.elapsed() < WAIT, "the launcher did not end");
    }
    rig.launcher.finish(Duration::ZERO);
    let start = Instant::now();
    let mut seen = Vec::new();
    while !seen.contains(&FromHub::Quit) && start.elapsed() < WAIT {
        seen.extend(main.poll());
    }
    assert!(seen.contains(&FromHub::Quit));
    assert!(rig.fake.0.borrow().stopped);
    assert!(
        rig.launcher
            .take_log()
            .contains(&"Window 7 did not close in time and was stopped".to_owned())
    );
    let saved = LayoutFile::new(&rig.file)
        .load()
        .expect("loads")
        .expect("saved");
    assert_eq!(
        saved.windows.len(),
        2,
        "saved at once, with the popped-out window"
    );
}

#[test]
fn a_window_saying_hello_changes_nothing_to_save() {
    let mut rig = Rig::new("hello", default_layout());
    let mut main = rig.window(WindowId::MAIN);
    rig.until(&mut main, is_layout);
    let start = Instant::now();
    while start.elapsed() < fusion_launch::SAVE_DELAY * 2 {
        rig.turn();
    }
    assert!(!rig.file.exists(), "nothing changed, so nothing is saved");
}

#[test]
fn a_line_that_is_not_a_message_is_logged() {
    let mut rig = Rig::new("bad", default_layout());
    let mut raw = Stream::connect(
        rig.name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("name"),
    )
    .expect("connects");
    raw.write_all(b"nope\n").expect("writes");
    rig.until_rig(|rig| !rig.log.is_empty());
    assert!(rig.log[0].contains("a line is not a message"));
}
