//! Native entry. The window is undecorated so the fluent2 title bar owns
//! the caption buttons.

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("rustTorrent")
            .with_inner_size([1100.0, 720.0])
            .with_decorations(false),
        ..Default::default()
    };
    eframe::run_native(
        "rustTorrent",
        options,
        Box::new(|_creation| Ok(Box::new(Shell(rtbit_egui::App::new())))),
    )
}

struct Shell(rtbit_egui::App);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}
