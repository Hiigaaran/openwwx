//! OpenWWX desktop app: load your card sets and play Raw Deal against the
//! computer or online.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod anim;
mod app;
mod art;
mod builder;
mod cards;
mod game;
mod o8c;
mod social;
mod table;
mod text;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("OpenWWX")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("OpenWWX", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
