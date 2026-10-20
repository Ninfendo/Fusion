//! The Fusion desktop app: one window with the dock.
//!
//! The layout and the theme are saved when the app closes, and every 30
//! seconds, and restored at the next start. Milestone 2 adds one process per
//! window.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fusion")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Fusion",
        options,
        Box::new(|cc| Ok(Box::new(fusion_ui_egui::FusionApp::new(cc)))),
    )
}
