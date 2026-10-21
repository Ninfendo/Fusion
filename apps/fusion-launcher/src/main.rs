//! The Fusion launcher: runs the hub and one `fusion-window` process per
//! window. Closing the main window ends the app.
//!
//! The layout is saved in Fusion's settings folder (see
//! [`fusion_launch::config_dir`]) and restored at the next start, popped-out
//! windows included. `fusion-window` must sit next to this program.
//!
//! With `--server ws://HOST:PORT` every window shows live data from the view
//! server; without it, demo data.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use fusion_client_core::workspace::default_layout;
use fusion_launch::{ChildProcesses, Launcher, LayoutFile, config_dir};
use fusion_pipe::{HubServer, hub_name};

/// How long one turn waits for messages before it checks the processes.
const TURN: Duration = Duration::from_millis(200);
/// How long windows get to close at the end before they are killed.
const GRACE: Duration = Duration::from_secs(3);

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
    let layout = match file.load() {
        Ok(Some(layout)) => layout,
        Ok(None) => default_layout(),
        Err(err) => {
            eprintln!("The saved layout could not be loaded ({err}); using the default.");
            default_layout()
        }
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
