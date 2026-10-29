//! Browser entry. Build with
//! `cargo build --manifest-path examples/fluent-mockup/Cargo.toml --target wasm32-unknown-unknown`.

#[cfg(target_arch = "wasm32")]
use eframe::wasm_bindgen::JsCast as _;

#[cfg(target_arch = "wasm32")]
fn main() {
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("mockup"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok());
    let options = eframe::WebOptions::default();
    let start = eframe::WebRunner::new();
    let runner = Box::new(|creation: &eframe::CreationContext<'_>| {
        rtbit_fluent_mockup::install_theme(&creation.egui_ctx);
        Ok(Box::new(App(rtbit_fluent_mockup::Mockup::new())) as Box<dyn eframe::App>)
    });
    wasm_bindgen_futures::spawn_local(async move {
        let _ = start
            .start(canvas.expect("canvas #mockup"), options, runner)
            .await;
    });
}

#[cfg(target_arch = "wasm32")]
struct App(rtbit_fluent_mockup::Mockup);

#[cfg(target_arch = "wasm32")]
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui, ui.input(|input| input.time as f32));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
