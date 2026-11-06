//! Phosphor icon setup and small helpers shared by the GUI.
//!
//! The regular Phosphor font is registered as a fallback for the proportional
//! and monospace families, so icons can be embedded straight into ordinary
//! labels and buttons: `format!("{} Save", icons::FLOPPY_DISK.as_str())`.

use crate::theme as th;
use egui::{RichText, WidgetInfo, WidgetType};
use egui_phosphor_icons::Icon;
use std::sync::Arc;

pub use egui_phosphor_icons::icons::*;

/// The bundled UI typeface. Inter ships under the SIL Open Font License; see
/// `assets/fonts/OFL.txt`.
const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

/// Register the UI and Phosphor fonts. Call once during startup.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor_icons::add_fonts(&mut fonts);

    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(egui::FontData::from_static(INTER_REGULAR)),
    );
    fonts.font_data.insert(
        "inter-semibold".to_owned(),
        Arc::new(egui::FontData::from_static(INTER_SEMIBOLD)),
    );

    // Inter is the primary face, with the egui defaults kept as fallbacks for
    // scripts and symbols Inter does not cover. Phosphor icons come last so
    // icon glyphs embedded in labels still resolve.
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    proportional.insert(0, "inter".to_owned());
    if !proportional.iter().any(|name| name == "phosphor-icons") {
        proportional.push("phosphor-icons".to_owned());
    }

    let monospace = fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default();
    monospace.push("inter".to_owned());
    if !monospace.iter().any(|name| name == "phosphor-icons") {
        monospace.push("phosphor-icons".to_owned());
    }

    // A real semibold face for headings and emphasis. `.strong()` alone can
    // only change the colour, so the display weight gets its own family.
    let proportional = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut semibold = vec!["inter-semibold".to_owned()];
    semibold.extend(proportional);
    fonts
        .families
        .insert(egui::FontFamily::Name(th::SEMIBOLD_FAMILY.into()), semibold);

    ctx.set_fonts(fonts);
}

/// The MusicWasp logo, decoded and downscaled. A custom skin may point at its
/// own image in `skins.json`; otherwise the embedded dark/light logo is used.
fn logo_rgba(size: u32, skin: crate::theme::Skin) -> Option<(usize, Vec<u8>)> {
    const DARK: &[u8] = include_bytes!("../../assets/MusicWasp-dark.png");
    const LIGHT: &[u8] = include_bytes!("../../assets/MusicWasp.png");
    let custom = skin.icon_path().and_then(|path| image::open(path).ok());
    let logo = match custom {
        Some(image) => image.into_rgba8(),
        None => {
            let bytes = if skin.icon_is_light() { LIGHT } else { DARK };
            image::load_from_memory(bytes).ok()?.into_rgba8()
        }
    };
    let logo = image::imageops::resize(&logo, size, size, image::imageops::FilterType::Lanczos3);
    Some((logo.width() as usize, logo.into_raw()))
}

/// A texture handle for the logo, e.g. for the toolbar.
pub fn logo(
    ctx: &egui::Context,
    skin: crate::theme::Skin,
    size: u32,
) -> Option<egui::TextureHandle> {
    let (width, rgba) = logo_rgba(size, skin)?;
    let color = egui::ColorImage::from_rgba_unmultiplied([width, width], &rgba);
    Some(ctx.load_texture(
        format!("musicwasp-logo-{}", skin.id()),
        color,
        egui::TextureOptions::LINEAR,
    ))
}

/// The logo as window icon data.
pub fn icon_data(skin: crate::theme::Skin) -> Option<egui::IconData> {
    let (width, rgba) = logo_rgba(256, skin)?;
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
    let response = ui.button(with(icon, label));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    response
}

/// An enabled/disabled button with a leading icon and a text label.
pub fn button_enabled(ui: &mut egui::Ui, enabled: bool, icon: Icon, label: &str) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(with(icon, label)));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
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
    let response = ui.add_enabled(enabled, button);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, tooltip));
    response.on_hover_text(tooltip)
}

/// A toggle rendered as a single icon, e.g. shuffle and repeat.
pub fn toggle(ui: &mut egui::Ui, selected: bool, icon: Icon, tooltip: &str) -> egui::Response {
    let response = ui.selectable_label(selected, text(icon, 15.0));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, tooltip));
    response.on_hover_text(tooltip)
}

/// A transport control. `primary` fills a circular button with the accent
/// colour; the others stay frameless until hovered.
pub fn transport(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    primary: bool,
    tooltip: &str,
) -> egui::Response {
    let diameter = if primary {
        (size + 20.0).max(36.0)
    } else {
        (size + 12.0).max(28.0)
    };
    let mut glyph = text(icon, size + if primary { 2.0 } else { 0.0 });
    if primary {
        glyph = glyph.color(th::current().bg_dark);
    }
    let mut button = egui::Button::new(glyph)
        .min_size(egui::vec2(diameter, diameter))
        .corner_radius(th::pill())
        .frame_when_inactive(primary);
    if primary {
        button = button.fill(th::accent());
    }
    let response = ui.add(button);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, tooltip));
    response.on_hover_text(tooltip)
}
