use eframe::egui;

struct MinimalApp;

impl eframe::App for MinimalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Test Minimal - ça marche !");
            if ui.button("Cliquez-moi").clicked() {
                println!("✅ Bouton cliqué !");
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    println!("🎬 Test GUI minimal...");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Test Minimal"),
        ..Default::default()
    };

    eframe::run_native(
        "Test",
        options,
        Box::new(|_cc| Ok(Box::new(MinimalApp))),
    )
}
