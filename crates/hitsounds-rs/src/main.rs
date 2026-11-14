#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod chrome;
#[cfg(windows)]
mod crash;
#[cfg(windows)]
mod editor;
#[cfg(windows)]
mod export_view;
#[cfg(windows)]
mod keys;
#[cfg(windows)]
mod mapsets;
#[cfg(windows)]
mod palette;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod views;
#[cfg(windows)]
mod widgets;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
fn main() {
    use eframe::egui::{ViewportBuilder, vec2};

    crash::install();
    update::tidy();
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("bundled app icon is a PNG");
    let size = vec2(theme::INITIAL_WIDTH, theme::INITIAL_HEIGHT);
    // QA runs can ask for a window size, to check the layout at its narrowest.
    #[cfg(debug_assertions)]
    let size = std::env::var("HITSOUNDS_RS_QA_SIZE")
        .ok()
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some(vec2(width.parse().ok()?, height.parse().ok()?))
        })
        .unwrap_or(size);
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_app_id("dev.hitsounds-rs.app")
            .with_inner_size(size)
            .with_min_inner_size(vec2(theme::MIN_WIDTH, theme::MIN_HEIGHT))
            .with_icon(icon.clone()),
        centered: true,
        persistence_path: worker::config_folder().map(|dir| dir.join("window.ron")),
        ..Default::default()
    };
    let result = eframe::run_native(
        "hitsounds-rs",
        options,
        Box::new(move |cc| Ok(Box::new(app::Studio::new(cc, &icon)?))),
    );
    match result {
        // D-84: a downloaded update is applied once the window is gone.
        Ok(()) => update::apply_on_exit(),
        Err(error) => {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("hitsounds-rs could not start")
                .set_description(error.to_string())
                .show();
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("hitsounds-rs requires Windows 10 or 11.");
}
