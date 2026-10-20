//! The Fusion app in the browser.
//!
//! The same `FusionApp` as the desktop app, drawn on a canvas. The layout and
//! the theme are saved in the browser's local storage. Build and serve it
//! with Trunk, from this folder: `trunk serve`.

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(canvas) = document
            .get_element_by_id("fusion-canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return;
        };
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(fusion_ui_egui::FusionApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(_) => loading
                    .set_inner_html("<p>Fusion could not start. The browser console says why.</p>"),
            }
        }
    });
}

/// On the desktop this binary only explains where it runs.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("fusion-web runs in the browser: cd apps/fusion-web && trunk serve");
}
