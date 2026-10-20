//! The Fusion launcher: runs the hub and one `fusion-window` process per
//! window. Closing the main window ends the app.
//!
//! The layout is saved in Fusion's settings folder (see
//! [`fusion_launch::config_dir`]) and restored at the next start, popped-out
//! windows included. `fusion-window` must sit next to this program.

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

/// `fusion-window`, next to this program.
fn window_program() -> std::io::Result<PathBuf> {
    let me = std::env::current_exe()?;
    Ok(me.with_file_name(format!("fusion-window{}", std::env::consts::EXE_SUFFIX)))
}
