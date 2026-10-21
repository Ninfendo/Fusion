//! The Fusion launcher: runs the hub and one `fusion-window` process per
//! window. Closing the main window ends the app.
//!
//! The layout is saved in Fusion's settings folder (see
//! [`fusion_launch::config_dir`]) and restored at the next start, popped-out
//! windows included. `fusion-window` must sit next to this program.
//!
//! With `--server ws://HOST:PORT` every window shows live data from the view
//! server, and the layout is kept there too: the server's copy wins at the
//! start, and the file stays as a local copy. Without it, demo data and the
//! file only.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use fusion_client_core::workspace::{LayoutChoice, SERVER_LAYOUT, choose_layout, default_layout};
use fusion_client_link::{RemoteEvent, RemoteLayouts};
use fusion_launch::{ChildProcesses, Launcher, LayoutFile, LayoutMirror, config_dir};
use fusion_layout::Layout;
use fusion_pipe::{HubServer, hub_name};

/// How long one turn waits for messages before it checks the processes.
const TURN: Duration = Duration::from_millis(200);
/// How long windows get to close at the end before they are killed.
const GRACE: Duration = Duration::from_secs(3);
/// How long the start waits for the view server's layout.
const LOAD_WAIT: Duration = Duration::from_secs(3);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Fusion could not start: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let view_server = server_arg(std::env::args().skip(1))?;
    let folder = config_dir(std::env::consts::OS, |name| std::env::var_os(name))
        .ok_or("there is no home folder to keep the layout in")?;
    let file = LayoutFile::new(folder.join("layout.json"));
    let local = || match file.load() {
        Ok(Some(layout)) => layout,
        Ok(None) => default_layout(),
        Err(err) => {
            eprintln!("The saved layout could not be loaded ({err}); using the default.");
            default_layout()
        }
    };
    let (layout, mirror) = match &view_server {
        Some(url) => {
            let mut remote = RemoteLayouts::connect(url, SERVER_LAYOUT, || {})?;
            let start = starting_layout(remote.wait_loaded(LOAD_WAIT), local);
            if let Some(note) = start.note {
                eprintln!("{note}");
            }
            if start.send_local
                && let Ok(json) = start.layout.to_json()
            {
                remote.save(json);
            }
            (start.layout, Some(ServerMirror(remote)))
        }
        None => (local(), None),
    };
    let name = hub_name(std::process::id());
    let server = HubServer::listen(&name)?;
    let window_program = window_program()?;
    // Each window keeps its place on the screen and its theme here.
    let windows = folder.join("windows");
    let processes = ChildProcesses::new(move |window| {
        let mut command = Command::new(&window_program);
        command
            .args([
                "--window",
                &window.to_string(),
                "--hub",
                &name,
                "--settings",
            ])
            .arg(windows.join(format!("{window}.ron")));
        if let Some(url) = &view_server {
            command.args(["--server", url]);
        }
        command
    });
    let mut launcher = Launcher::new(layout, server, processes, file);
    if let Some(mirror) = mirror {
        launcher = launcher.with_mirror(Box::new(mirror));
    }
    launcher.start();
    while launcher.turn(TURN) {
        print_log(&mut launcher);
    }
    launcher.finish(GRACE);
    print_log(&mut launcher);
    Ok(())
}

fn print_log<P: fusion_launch::WindowProcesses>(launcher: &mut Launcher<P>) {
    for line in launcher.take_log() {
        eprintln!("{line}");
    }
}

/// The layout to start with, and what to do about it.
struct Start {
    layout: Layout,
    /// The server has no layout yet: send it the local one.
    send_local: bool,
    /// What to tell the user, if anything.
    note: Option<String>,
}

/// The view server's layout when it has one, else the local one (see
/// [`choose_layout`]). A server that did not answer in time keeps its copy.
fn starting_layout(
    from_server: Option<Result<Option<String>, String>>,
    local: impl FnOnce() -> Layout,
) -> Start {
    let choice = match from_server {
        Some(answer) => choose_layout(answer, false),
        None => LayoutChoice::KeepLocal(
            "The view server did not answer in time; using the local layout.".to_owned(),
        ),
    };
    match choice {
        LayoutChoice::UseServer(layout) => Start {
            layout,
            send_local: false,
            note: None,
        },
        LayoutChoice::SendLocal => Start {
            layout: local(),
            send_local: true,
            note: None,
        },
        LayoutChoice::KeepLocal(note) => Start {
            layout: local(),
            send_local: false,
            note: Some(note),
        },
    }
}

/// The view server as the launcher's layout mirror.
struct ServerMirror(RemoteLayouts);

impl LayoutMirror for ServerMirror {
    fn save(&mut self, json: String) {
        self.0.save(json);
    }

    fn poll(&mut self) -> Vec<String> {
        failures(self.0.poll())
    }

    fn settle(&mut self, wait: Duration) -> Vec<String> {
        failures(self.0.settle(wait))
    }
}

/// Lines for the log: the saves the server refused.
fn failures(events: Vec<RemoteEvent>) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|event| match event {
            RemoteEvent::SaveFailed(why) => {
                Some(format!("The view server did not keep the layout: {why}"))
            }
            RemoteEvent::Loaded(_) => None,
        })
        .collect()
}

/// The view server's address, from `--server ws://HOST:PORT`, if given.
fn server_arg(mut args: impl Iterator<Item = String>) -> Result<Option<String>, String> {
    let mut server = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = Some(args.next().ok_or("--server needs an address")?),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(server)
}

/// `fusion-window`, next to this program.
fn window_program() -> std::io::Result<PathBuf> {
    let me = std::env::current_exe()?;
    Ok(me.with_file_name(format!("fusion-window{}", std::env::consts::EXE_SUFFIX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(text: &str) -> Result<Option<String>, String> {
        server_arg(text.split_whitespace().map(str::to_owned))
    }

    /// A layout unlike the default one.
    fn local() -> Layout {
        Layout::new("Local")
    }

    #[test]
    fn the_server_s_layout_wins_when_it_has_one() {
        let json = default_layout().to_json().expect("json");
        let start = starting_layout(Some(Ok(Some(json))), local);
        assert_eq!(start.layout, default_layout());
        assert!(!start.send_local);
        assert_eq!(start.note, None);
    }

    #[test]
    fn the_local_layout_goes_up_only_when_the_server_has_none() {
        let start = starting_layout(Some(Ok(None)), local);
        assert_eq!(
            (start.layout, start.send_local, start.note),
            (local(), true, None)
        );
        for answer in [
            None,
            Some(Err("down".to_owned())),
            Some(Ok(Some("{".to_owned()))),
        ] {
            let start = starting_layout(answer, local);
            assert_eq!(start.layout, local());
            assert!(!start.send_local);
            assert!(start.note.is_some_and(|n| n.contains("local")));
        }
    }

    #[test]
    fn refused_saves_are_logged() {
        let events = vec![
            RemoteEvent::Loaded(Ok(None)),
            RemoteEvent::SaveFailed("full".to_owned()),
        ];
        assert_eq!(
            failures(events),
            ["The view server did not keep the layout: full"]
        );
    }

    #[test]
    fn the_server_is_optional() {
        assert_eq!(server(""), Ok(None));
        assert_eq!(
            server("--server ws://h:7070"),
            Ok(Some("ws://h:7070".to_owned()))
        );
        assert!(server("--server").is_err());
        assert!(server("--fast").is_err());
    }
}
