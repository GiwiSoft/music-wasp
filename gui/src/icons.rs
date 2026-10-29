//! Phosphor icon setup and small helpers shared by the GUI.
//!
//! The regular Phosphor font is registered as a fallback for the proportional
//! and monospace families, so icons can be embedded straight into ordinary
//! labels and buttons: `format!("{} Save", icons::FLOPPY_DISK.as_str())`.

use crate::theme as th;
use egui::RichText;
use egui_phosphor_icons::Icon;

pub use egui_phosphor_icons::icons::*;

/// Register the Phosphor fonts. Call once during startup.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor_icons::add_fonts(&mut fonts);

    // `add_fonts` only creates named families; add the regular glyphs as a
    // fallback so icons work inside normal text.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("phosphor-icons".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

/// The MusicWasp logo, decoded and downscaled. Light skins use the light asset.
fn logo_rgba(size: u32, light: bool) -> Option<(usize, Vec<u8>)> {
    // Dark skins get the logo with white lettering, light skins the dark one.
    const DARK: &[u8] = include_bytes!("../../assets/MusicWasp-dark.png");
    const LIGHT: &[u8] = include_bytes!("../../assets/MusicWasp.png");
    let bytes = if light { LIGHT } else { DARK };
    let logo = image::load_from_memory(bytes).ok()?.into_rgba8();
    let logo = image::imageops::resize(&logo, size, size, image::imageops::FilterType::Lanczos3);
    Some((logo.width() as usize, logo.into_raw()))
}

/// A texture handle for the logo, e.g. for the toolbar.
pub fn logo(ctx: &egui::Context, light: bool, size: u32) -> Option<egui::TextureHandle> {
    let (width, rgba) = logo_rgba(size, light)?;
    let color = egui::ColorImage::from_rgba_unmultiplied([width, width], &rgba);
    let name = if light {
        "musicwasp-logo-light"
    } else {
        "musicwasp-logo"
    };
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

/// The logo as window icon data.
pub fn icon_data(light: bool) -> Option<egui::IconData> {
    let (width, rgba) = logo_rgba(256, light)?;
    Some(egui::IconData {
        rgba,
        width: width as u32,
        height: width as u32,
    })
}

/// An icon glyph as styled text, for labels and painted text.
pub fn text(icon: Icon, size: f32) -> RichText {
    RichText::new(icon.as_str()).size(size)
}

/// `icon label` string for buttons and window titles.
pub fn with(icon: Icon, label: &str) -> String {
    format!("{} {label}", icon.as_str())
}

/// A button with a leading icon and a text label.
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    ui.button(with(icon, label))
}

/// An enabled/disabled button with a leading icon and a text label.
pub fn button_enabled(ui: &mut egui::Ui, enabled: bool, icon: Icon, label: &str) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(with(icon, label)))
}

/// A compact icon-only button (with a tooltip), for dense rows.
pub fn compact(ui: &mut egui::Ui, icon: Icon, tooltip: &str) -> egui::Response {
    compact_enabled(ui, true, icon, tooltip)
}

/// A compact icon-only button that can be disabled.
pub fn compact_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: Icon,
    tooltip: &str,
) -> egui::Response {
    let button = egui::Button::new(text(icon, 14.0)).min_size(egui::vec2(24.0, 22.0));
    ui.add_enabled(enabled, button).on_hover_text(tooltip)
}

/// A toggle rendered as a single icon, e.g. shuffle and repeat.
pub fn toggle(ui: &mut egui::Ui, selected: bool, icon: Icon, tooltip: &str) -> egui::Response {
    ui.selectable_label(selected, text(icon, 15.0))
        .on_hover_text(tooltip)
}

/// A transport control. `primary` fills the button with the accent colour.
pub fn transport(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    primary: bool,
    tooltip: &str,
) -> egui::Response {
    let mut glyph = text(icon, size);
    if primary {
        glyph = glyph.color(th::current().bg_dark);
    }
    let mut button = egui::Button::new(glyph).min_size(egui::vec2(size + 18.0, 30.0));
    if primary {
        button = button.fill(th::accent());
    }
    ui.add(button).on_hover_text(tooltip)
}
