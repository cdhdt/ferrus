//! Ferrus graphical front-end.
//!
//! GUI framework: **egui via eframe** (see `docs/adr/0001-gui-framework.md`).
//!
//! Phase 0 scope: this is a **window shell only**. It opens an empty eframe
//! window titled "Ferrus" with a placeholder label. There is deliberately no
//! flow logic and no call into `ferrus-core` yet — the burn UI is Phase 5. The
//! point is only that the crate builds and a window opens.

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferrus")
            .with_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Ferrus",
        options,
        Box::new(|_cc| Ok(Box::<FerrusApp>::default())),
    )
}

/// Placeholder application state. Fields will arrive with the Phase 5 flow.
#[derive(Default)]
struct FerrusApp;

impl eframe::App for FerrusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Ferrus");
            ui.label("GUI placeholder — nothing wired up yet (Phase 5).");
        });
    }
}
