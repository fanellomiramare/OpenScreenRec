//! OpenScreenRec — a free and open-source screen recorder, 100% Rust.
//!
//! M0 build: application shell only. The recording pipeline lands
//! milestone by milestone (see the roadmap in the README).

use eframe::egui;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!(
        "starting {} v{}",
        osr_common::APP_NAME,
        env!("CARGO_PKG_VERSION")
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 300.0])
            .with_min_inner_size([380.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        osr_common::APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(App))),
    )
}

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.vertical_centered(|ui| {
            ui.add_space(28.0);
            ui.heading(osr_common::APP_NAME);
            ui.add_space(4.0);
            ui.weak(format!(
                "v{} — 100% Rust, MIT licensed",
                env!("CARGO_PKG_VERSION")
            ));
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(18.0);
            ui.label("Early scaffolding build (M0).");
            ui.label("The recording pipeline lands milestone by milestone:");
            ui.weak("M1 video capture · M2 audio · M3 controls");
            ui.add_space(28.0);
            ui.weak("Your recordings never leave your machine.");
        });
    }
}
