//! rustTorrent Fluent 2 mockup.
//!
//! Native: `cargo run -p rtbit-fluent-mockup`. Wasm: `trunk serve` in this crate.
//!
//! Native flags, for screenshots: `--screen login|torrents|settings|rss|log|add|delete`,
//! `--dark`, `--compact`.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use rtbit_fluent_mockup::{Launch, MockupApp};

    let launch = Launch::from_args(std::env::args().skip(1));
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("rustTorrent — Fluent mockup")
            .with_inner_size([1440.0, 900.0]),
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "rustTorrent Fluent mockup",
        native,
        Box::new(move |cc| Ok(Box::new(MockupApp::new(&cc.egui_ctx, launch)))),
    ) {
        eprintln!("rtbit-fluent-mockup: {error}");
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = start_web().await {
            log::error!("mockup failed to start: {error}");
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn start_web() -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast as _;
    use rtbit_fluent_mockup::{Launch, MockupApp};

    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or("no document")?;
    let canvas = document
        .get_element_by_id("the_canvas_id")
        .ok_or("no #the_canvas_id element")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "#the_canvas_id is not a canvas")?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(MockupApp::new(&cc.egui_ctx, Launch::default())))),
        )
        .await
        .map_err(|error| format!("{error:?}"))
}
