//! The Fusion desktop window.
//!
//! Started on its own, it is the whole app in one window: the layout and
//! the theme are saved when it closes, and every 30 seconds.
//!
//! Started by `fusion-launcher` with `--window N --hub NAME --settings
//! FILE`, it is one window of the workspace. The launcher keeps the layout;
//! the window keeps only its theme and its place on the screen, in FILE.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::{egui, egui_wgpu, wgpu};
use fusion_layout::WindowId;
use fusion_pipe::PipeBus;
use fusion_ui_egui::{FusionApp, THEME_KEY};

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Start {
    /// The whole app in one window.
    Alone,
    /// One window of the launcher's workspace.
    InHub {
        window: WindowId,
        hub: String,
        settings: Option<PathBuf>,
    },
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Start, String> {
    let (mut window, mut hub, mut settings) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--window" => {
                let number = args.next().and_then(|n| n.parse().ok());
                window = Some(WindowId(number.ok_or("--window needs a number")?));
            }
            "--hub" => hub = Some(args.next().ok_or("--hub needs a name")?),
            "--settings" => {
                settings = Some(PathBuf::from(args.next().ok_or("--settings needs a file")?));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    match (window, hub, settings) {
        (None, None, None) => Ok(Start::Alone),
        (Some(window), Some(hub), settings) => Ok(Start::InHub {
            window,
            hub,
            settings,
        }),
        _ => Err("--window and --hub go together, and --settings needs them".to_owned()),
    }
}

/// The graphics backends to use: OpenGL on Windows and Linux, Metal on
/// macOS, unless `WGPU_BACKEND` names others.
///
/// With one process per window, memory per window counts. Measured on
/// Windows, release build, 2026-10-20: about 90 MB of memory per window on
/// OpenGL, 240 MB on DX12 or Vulkan, and 290 MB with every backend loaded.
/// OpenGL used less CPU time too. wgpu has no native OpenGL on macOS.
fn backends(os: &str) -> wgpu::Backends {
    wgpu::Backends::from_env().unwrap_or(if os == "macos" {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::GL
    })
}

fn options() -> eframe::NativeOptions {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends(std::env::consts::OS);
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fusion")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    let start = parse(std::env::args().skip(1)).unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(2)
    });
    match start {
        Start::Alone => eframe::run_native(
            "Fusion",
            options(),
            Box::new(|cc| Ok(Box::new(FusionApp::new(cc)))),
        ),
        // Each window keeps its own place on the screen, in its own file.
        Start::InHub {
            window,
            hub,
            settings,
        } => eframe::run_native(
            &format!("Fusion window {window}"),
            eframe::NativeOptions {
                persistence_path: settings,
                ..options()
            },
            Box::new(move |cc| {
                let ctx = cc.egui_ctx.clone();
                let bus = PipeBus::connect(&hub, window, move || ctx.request_repaint())?;
                let theme = cc.storage.and_then(|s| s.get_string(THEME_KEY));
                Ok(Box::new(FusionApp::with_bus(
                    Box::new(bus),
                    window,
                    theme.as_deref(),
                )))
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_text(text: &str) -> Result<Start, String> {
        parse(text.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn each_system_gets_its_light_backend() {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            assert_eq!(backends("windows"), wgpu::Backends::GL);
            assert_eq!(backends("linux"), wgpu::Backends::GL);
            assert_eq!(backends("macos"), wgpu::Backends::METAL);
        }
    }

    #[test]
    fn the_command_line_picks_the_start() {
        assert_eq!(parse_text(""), Ok(Start::Alone));
        assert_eq!(
            parse_text("--window 2 --hub fusion-hub-7.sock"),
            Ok(Start::InHub {
                window: WindowId(2),
                hub: "fusion-hub-7.sock".to_owned(),
                settings: None,
            })
        );
        assert_eq!(
            parse_text("--window 3 --hub h --settings w/3.ron"),
            Ok(Start::InHub {
                window: WindowId(3),
                hub: "h".to_owned(),
                settings: Some(PathBuf::from("w/3.ron")),
            })
        );
        assert!(parse_text("--settings w/3.ron").is_err());
        assert!(parse_text("--window 3 --hub h --settings").is_err());
        assert!(parse_text("--window two --hub x").is_err());
        assert!(parse_text("--window 2").is_err());
        assert!(parse_text("--hub").is_err());
        assert!(parse_text("--fast").is_err());
    }
}
