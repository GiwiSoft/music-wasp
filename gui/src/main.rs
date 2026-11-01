use musicwasp_gui::app::App;
use musicwasp_gui::{icons, settings::Settings, theme};

fn main() -> eframe::Result<()> {
    maybe_force_x11();
    theme::load_user_skins();
    // Screenshot runs need a predictable canvas and must not inherit a session.
    let screenshot = std::env::var_os("MUSICWASP_SCREENSHOT").is_some();
    let settings = if screenshot {
        None
    } else {
        Some(Settings::load(&Settings::path()))
    };
    let saved_window = settings
        .as_ref()
        .and_then(|settings| settings.session.window);

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
    let skin = settings
        .as_ref()
        .map(|settings| settings.skin)
        .unwrap_or_default();
    if let Some(icon) = icons::icon_data(skin) {
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
