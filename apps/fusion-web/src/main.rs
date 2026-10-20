//! The Fusion app in the browser.
//!
//! The same `FusionApp` as the desktop app, drawn on a canvas. The page
//! shows the main window; `?window=N` shows a popped-out window. The main
//! tab runs the hub and saves the layout in the browser's local storage.
//! Build and serve it with Trunk, from this folder: `trunk serve`.

#[cfg(target_arch = "wasm32")]
mod web_bus;

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
        let query = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default();
        let window = fusion_hub::window_from_query(&query);
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let ctx = cc.egui_ctx.clone();
                    let bus = web_bus::WebBus::new(window, move || ctx.request_repaint())
                        .map_err(|err| format!("the tab channel did not open: {err:?}"))?;
                    let theme = cc
                        .storage
                        .and_then(|s| s.get_string(fusion_ui_egui::THEME_KEY));
                    Ok(Box::new(fusion_ui_egui::FusionApp::with_bus(
                        Box::new(bus),
                        window,
                        theme.as_deref(),
                    )))
                }),
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
