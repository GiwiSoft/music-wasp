//! MusicWasp skins.
//!
//! A skin is a named palette applied to egui. Colours are read through
//! [`accent`] and friends so widgets that paint themselves (album art
//! placeholders, drag markers, equalizer curve) follow the active skin too.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};

/// Available skins, persisted in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Skin {
    /// Default dark skin built around the MusicWasp green.
    #[default]
    #[serde(alias = "amber")]
    Wasp,
    Midnight,
    Forest,
    Mono,
    #[serde(alias = "paper")]
    Light,
}

impl Skin {
    pub const ALL: [Skin; 5] = [
        Skin::Wasp,
        Skin::Midnight,
        Skin::Forest,
        Skin::Mono,
        Skin::Light,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Skin::Wasp => "MusicWasp Dark",
            Skin::Midnight => "Midnight (dark)",
            Skin::Forest => "Forest (dark)",
            Skin::Mono => "Mono (dark)",
            Skin::Light => "MusicWasp Light",
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Skin::Light)
    }

    pub fn palette(self) -> Palette {
        match self {
            Skin::Wasp => Palette {
                bg_dark: Color32::from_rgb(22, 26, 22),
                bg_panel: Color32::from_rgb(29, 34, 29),
                bg_row_alt: Color32::from_rgb(34, 40, 34),
                bg_selected: Color32::from_rgb(45, 72, 38),
                accent: Color32::from_rgb(90, 213, 27),
                text_dim: Color32::from_rgb(150, 160, 150),
                text: Color32::from_rgb(224, 232, 223),
                extreme_bg: Color32::from_rgb(16, 19, 16),
                widget_inactive: Color32::from_rgb(45, 52, 45),
                widget_weak: Color32::from_rgb(39, 45, 39),
                widget_hover: Color32::from_rgb(61, 71, 60),
            },
            Skin::Midnight => Palette {
                bg_dark: Color32::from_rgb(16, 20, 30),
                bg_panel: Color32::from_rgb(22, 28, 42),
                bg_row_alt: Color32::from_rgb(27, 34, 50),
                bg_selected: Color32::from_rgb(36, 66, 104),
                accent: Color32::from_rgb(107, 176, 255),
                text_dim: Color32::from_rgb(142, 158, 184),
                text: Color32::from_rgb(222, 232, 247),
                extreme_bg: Color32::from_rgb(11, 14, 22),
                widget_inactive: Color32::from_rgb(38, 48, 68),
                widget_weak: Color32::from_rgb(31, 40, 58),
                widget_hover: Color32::from_rgb(51, 64, 90),
            },
            Skin::Forest => Palette {
                bg_dark: Color32::from_rgb(20, 26, 22),
                bg_panel: Color32::from_rgb(27, 34, 29),
                bg_row_alt: Color32::from_rgb(32, 40, 34),
                bg_selected: Color32::from_rgb(48, 78, 58),
                accent: Color32::from_rgb(126, 205, 130),
                text_dim: Color32::from_rgb(147, 163, 150),
                text: Color32::from_rgb(223, 234, 224),
                extreme_bg: Color32::from_rgb(14, 18, 15),
                widget_inactive: Color32::from_rgb(41, 51, 44),
                widget_weak: Color32::from_rgb(34, 43, 37),
                widget_hover: Color32::from_rgb(57, 70, 60),
            },
            Skin::Mono => Palette {
                bg_dark: Color32::from_rgb(23, 23, 23),
                bg_panel: Color32::from_rgb(31, 31, 31),
                bg_row_alt: Color32::from_rgb(37, 37, 37),
                bg_selected: Color32::from_rgb(66, 66, 66),
                accent: Color32::from_rgb(214, 214, 214),
                text_dim: Color32::from_rgb(148, 148, 148),
                text: Color32::from_rgb(231, 231, 231),
                extreme_bg: Color32::from_rgb(16, 16, 16),
                widget_inactive: Color32::from_rgb(48, 48, 48),
                widget_weak: Color32::from_rgb(41, 41, 41),
                widget_hover: Color32::from_rgb(66, 66, 66),
            },
            Skin::Light => Palette {
                bg_dark: Color32::from_rgb(240, 241, 236),
                bg_panel: Color32::from_rgb(250, 251, 247),
                bg_row_alt: Color32::from_rgb(234, 236, 229),
                bg_selected: Color32::from_rgb(210, 232, 196),
                accent: Color32::from_rgb(58, 143, 16),
                text_dim: Color32::from_rgb(107, 102, 93),
                text: Color32::from_rgb(32, 31, 29),
                extreme_bg: Color32::from_rgb(226, 222, 213),
                widget_inactive: Color32::from_rgb(221, 216, 206),
                widget_weak: Color32::from_rgb(230, 226, 218),
                widget_hover: Color32::from_rgb(208, 202, 190),
            },
        }
    }
}

/// All colours a skin needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub bg_dark: Color32,
    pub bg_panel: Color32,
    pub bg_row_alt: Color32,
    pub bg_selected: Color32,
    pub accent: Color32,
    pub text_dim: Color32,
    pub text: Color32,
    pub extreme_bg: Color32,
    pub widget_inactive: Color32,
    pub widget_weak: Color32,
    pub widget_hover: Color32,
}

static ACTIVE: OnceLock<RwLock<Palette>> = OnceLock::new();

fn active() -> &'static RwLock<Palette> {
    ACTIVE.get_or_init(|| RwLock::new(Skin::default().palette()))
}

/// Palette currently painted by the UI.
pub fn current() -> Palette {
    *active()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn accent() -> Color32 {
    current().accent
}
pub fn text() -> Color32 {
    current().text
}
pub fn text_dim() -> Color32 {
    current().text_dim
}
pub fn bg_row_alt() -> Color32 {
    current().bg_row_alt
}
pub fn bg_selected() -> Color32 {
    current().bg_selected
}

/// Install `skin` and rebuild the egui visuals.
pub fn apply(ctx: &egui::Context, skin: Skin) {
    let palette = skin.palette();
    *active()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = palette;

    let mut v = if skin.is_light() {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    v.override_text_color = Some(palette.text);
    v.panel_fill = palette.bg_dark;
    v.window_fill = palette.bg_panel;
    v.extreme_bg_color = palette.extreme_bg;
    v.faint_bg_color = palette.bg_row_alt;
    v.selection.bg_fill = palette.bg_selected;
    v.selection.stroke = Stroke::new(1.0_f32, palette.accent);
    v.hyperlink_color = palette.accent;
    v.window_stroke = Stroke::new(1.0_f32, palette.widget_hover);
    v.widgets.noninteractive.bg_fill = palette.bg_panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.widget_weak);
    v.widgets.inactive.bg_fill = palette.widget_inactive;
    v.widgets.inactive.weak_bg_fill = palette.widget_weak;
    v.widgets.hovered.bg_fill = palette.widget_hover;
    v.widgets.active.bg_fill = palette.accent;
    v.widgets.active.weak_bg_fill = palette.accent;
    v.widgets.active.fg_stroke = Stroke::new(
        1.0_f32,
        if skin.is_light() {
            palette.bg_panel
        } else {
            palette.text
        },
    );
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, palette.accent);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }

    ctx.set_theme(if skin.is_light() {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.scroll.bar_width = 10.0;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skin_has_a_distinct_palette() {
        let mut seen = Vec::new();
        for skin in Skin::ALL {
            let palette = skin.palette();
            assert!(!seen.contains(&palette), "{:?} reuses a palette", skin);
            seen.push(palette);
        }
    }

    #[test]
    fn applying_a_skin_updates_the_shared_palette() {
        let ctx = egui::Context::default();
        apply(&ctx, Skin::Midnight);
        assert_eq!(accent(), Skin::Midnight.palette().accent);
        assert_eq!(text(), Skin::Midnight.palette().text);

        apply(&ctx, Skin::Light);
        assert_eq!(accent(), Skin::Light.palette().accent);
        assert!(Skin::Light.is_light());

        // Leave the default behind for any other test that paints.
        apply(&ctx, Skin::default());
    }
}
