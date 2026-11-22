#![windows_subsystem = "windows"]
mod catalog;
mod db;
mod glass;
mod ssh;
mod ui;
mod update;
mod worker;
pub const APP_NAME: &str = "rkn-guard-panel";
fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([1080., 710.])
        .with_min_inner_size([760., 520.]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("rkn-mark.png")) {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        APP_NAME,
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)?))),
    )
}
