//! hexe desktop app (egui via eframe). No custom wgpu pass yet.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("hexe");
        ui.label("Houdini port: nothing implemented yet.");
    }
}

fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("hexe {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    eframe::run_native("hexe", eframe::NativeOptions::default(), Box::new(|_cc| Ok(Box::new(App))))
}
