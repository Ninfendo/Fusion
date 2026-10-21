//! How long the app takes to make one frame, on live data.
//!
//! ```text
//! cargo run --release -p fusion-server -- --venue synthetic --instruments 500
//! cargo run --release -p fusion-ui-egui --example frame_time -- ws://127.0.0.1:7070 20
//! ```
//!
//! Runs the app with the default layout for the given seconds, at 60
//! frames a second in real time, so the data arrives at its real pace. A
//! frame is the app's work plus egui's: layout, and the shapes turned into
//! triangles. Drawing them on the GPU is not in it. Prints the frame times;
//! at 60 frames a second a frame has 16.7 ms.

use std::time::{Duration, Instant};

use eframe::App as _;
use fusion_client_core::{MarketData as _, MarketFeed as _};
use fusion_client_link::LiveFeed;
use fusion_ui_egui::FusionApp;

const FRAME: Duration = Duration::from_micros(16_667);
/// Frames at the start that are not counted: fonts load, books arrive.
const WARM_UP: usize = 120;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "ws://127.0.0.1:7070".to_owned());
    let seconds: usize = args.next().map_or(Ok(20), |s| s.parse())?;
    let frames = WARM_UP + seconds * 60;

    // A second feed, to count the instruments the app sees.
    let mut counter = LiveFeed::connect(&url, || {})?;
    let mut app = FusionApp::from_saved(None, None);
    app.set_market(Box::new(LiveFeed::connect(&url, || {})?));
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));

    let start = Instant::now();
    let mut times = Vec::with_capacity(frames);
    for n in 0..frames {
        let due = start + FRAME * u32::try_from(n)?;
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(start.elapsed().as_secs_f64()),
            predicted_dt: FRAME.as_secs_f32(),
            ..Default::default()
        };
        let began = Instant::now();
        let output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        let triangles = ctx.tessellate(output.shapes, output.pixels_per_point);
        let took = began.elapsed();
        std::hint::black_box(triangles);
        counter.update(0.0);
        if n >= WARM_UP {
            times.push(took);
        }
    }
    report(&mut times, counter.symbols().len(), app.market_status());
    Ok(())
}

fn report(
    times: &mut [Duration],
    instruments: usize,
    status: Option<&fusion_client_core::FeedStatus>,
) {
    times.sort_unstable();
    let at = |share: f64| {
        let last = times.len().saturating_sub(1);
        // The index of a share of the sorted times, rounded down.
        let index = (share * last as f64) as usize;
        times.get(index).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    };
    let over = times.iter().filter(|t| **t > FRAME).count();
    println!(
        "{} frames, {instruments} instruments, market {}",
        times.len(),
        status.map_or_else(|| "unknown".to_owned(), ToString::to_string)
    );
    println!(
        "frame time ms: median {:.2}, 95% {:.2}, 99% {:.2}, worst {:.2}; over 16.7 ms: {over}",
        at(0.5),
        at(0.95),
        at(0.99),
        at(1.0)
    );
}
