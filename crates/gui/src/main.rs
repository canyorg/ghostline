//! GhostLine desktop app (Linux, Windows).
//!
//! Pure Rust (egui): no web view, no JavaScript, no HTML — message text is only
//! ever drawn as plain text. No messages, settings or history are written to disk;
//! only the built-in Tor keeps its guard/directory state (`GHOSTLINE_AMNESIC=1` → none).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod i18n;
mod matrix;
mod protect;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("GhostLine")
            .with_app_id("ghostline")
            .with_inner_size([1100.0, 740.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "GhostLine",
        options,
        Box::new(|cc| {
            let blocked = protect::exclude_from_capture(cc);
            Ok(Box::new(app::GhostApp::new(cc, blocked)))
        }),
    )
}
