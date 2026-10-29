//! Native window for the static mockup. Run with
//! `cargo run --manifest-path examples/fluent-mockup/Cargo.toml`.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("rustTorrent mockup")
            .with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rustTorrent mockup",
        options,
        Box::new(|creation| {
            rtbit_fluent_mockup::install_theme(&creation.egui_ctx);
            Ok(Box::new(Shell(rtbit_fluent_mockup::Mockup::new())))
        }),
    )
}

#[cfg(not(target_arch = "wasm32"))]
struct Shell(rtbit_fluent_mockup::Mockup);

#[cfg(not(target_arch = "wasm32"))]
impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui, ui.input(|input| input.time as f32));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
