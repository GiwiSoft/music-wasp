mod app;
mod equalizer;
mod icons;
#[cfg(target_os = "linux")]
mod mpris;
mod settings;
mod theme;
mod ui;

use app::App;

fn main() -> eframe::Result<()> {
    prefer_x11_for_file_drop();
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

/// winit 0.30 has no file drag and drop on its native Wayland backend. When
/// XWayland is available, prefer X11 so drops from the file manager work.
/// Set `MUSICWASP_NATIVE_WAYLAND=1` to keep the Wayland backend.
fn prefer_x11_for_file_drop() {
    let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var_os("WAYLAND_SOCKET").is_some();
    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    let opt_out = std::env::var_os("MUSICWASP_NATIVE_WAYLAND").is_some();
    if on_wayland && has_x11 && !opt_out {
        eprintln!(
            "MusicWasp: using X11 (XWayland) so file drag-and-drop works; \
             set MUSICWASP_NATIVE_WAYLAND=1 to keep native Wayland."
        );
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    }
}
