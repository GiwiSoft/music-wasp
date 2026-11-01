mod app;
mod equalizer;
mod icons;
#[cfg(target_os = "linux")]
mod mpris;
mod settings;
mod theme;
mod ui;
#[cfg(test)]
mod ui_tests;

use app::App;

fn main() -> eframe::Result<()> {
    maybe_force_x11();
    // Screenshot runs need a predictable canvas and must not inherit a session.
    let screenshot = std::env::var_os("MUSICWASP_SCREENSHOT").is_some();
    let saved_window = if screenshot {
        None
    } else {
        crate::settings::Settings::load(&crate::settings::Settings::path())
            .session
            .window
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([900.0, 560.0])
        .with_drag_and_drop(true)
        // Match the `StartupWMClass` in `packaging/musicwasp.desktop`, so the
        // desktop shows the launcher icon for a running window.
        .with_app_id("musicwasp")
        .with_title("MusicWasp");
    if let Some(window) = saved_window {
        viewport = viewport.with_inner_size([window.width.max(900.0), window.height.max(560.0)]);
        if let (Some(x), Some(y)) = (window.x, window.y) {
            viewport = viewport.with_position([x, y]);
        }
    }
    if let Some(icon) = icons::icon_data(false) {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "MusicWasp",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// Native Wayland is the default and supports file drops through the patched
/// winit backend. `MUSICWASP_X11=1` remains as a one-launch escape hatch for
/// compositors with broken drag-and-drop or clipboard behaviour.
fn maybe_force_x11() {
    let force_x11 = std::env::var_os("MUSICWASP_X11").is_some_and(|value| {
        let value = value.to_string_lossy();
        !value.is_empty() && value != "0"
    });
    if force_x11 && std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        eprintln!("MusicWasp: MUSICWASP_X11=1, using X11/XWayland");
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    }
}
