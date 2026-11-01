//! MusicWasp skins.
//!
//! A skin is a named palette applied to egui. Colours are read through
//! [`accent`] and friends so widgets that paint themselves (album art
//! placeholders, drag markers, equalizer curve) follow the active skin too.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// A skin identifier persisted in `settings.json`.
///
/// The list of skins, labels, colours, backdrops and icons lives in
/// `assets/skins.json`. A user copy in the data directory can override built-in
/// entries or add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Skin {
    id: &'static str,
}

impl Default for Skin {
    fn default() -> Self {
        Self::WASP
    }
}

impl Serialize for Skin {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id)
    }
}

impl<'de> Deserialize<'de> for Skin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::from_id(&id))
    }
}

impl Skin {
    #[allow(dead_code)]
    pub const WASP: Skin = Skin { id: "wasp" };
    #[allow(dead_code)]
    pub const MIDNIGHT: Skin = Skin { id: "midnight" };
    #[allow(dead_code)]
    pub const FOREST: Skin = Skin { id: "forest" };
    #[allow(dead_code)]
    pub const MONO: Skin = Skin { id: "mono" };
    #[allow(dead_code)]
    pub const HIGH_CONTRAST: Skin = Skin {
        id: "high_contrast",
    };
    #[allow(dead_code)]
    pub const LIQUID_GLASS_DARK: Skin = Skin {
        id: "liquid_glass_dark",
    };
    #[allow(dead_code)]
    pub const LIQUID_GLASS_LIGHT: Skin = Skin {
        id: "liquid_glass_light",
    };
    #[allow(dead_code)]
    pub const LIGHT: Skin = Skin { id: "light" };

    /// All skins in display order.
    pub fn all() -> Vec<Skin> {
        skins()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|definition| Skin { id: definition.id })
            .collect()
    }

    /// Resolve an id or alias. Unknown ids fall back to the default skin.
    pub fn from_id(id: &str) -> Self {
        let definitions = skins()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        definitions
            .iter()
            .find(|definition| {
                definition.id == id || definition.aliases.iter().any(|alias| *alias == id)
            })
            .map(|definition| Skin { id: definition.id })
            .unwrap_or(Self::WASP)
    }

    pub fn id(self) -> &'static str {
        self.id
    }

    fn definition(self) -> SkinDefinition {
        skins()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|definition| definition.id == self.id)
            .cloned()
            .or_else(|| {
                skins()
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .find(|definition| definition.id == Skin::WASP.id)
                    .cloned()
            })
            .unwrap_or_else(SkinDefinition::fallback)
    }

    pub fn label(self) -> &'static str {
        self.definition().label
    }

    pub fn is_light(self) -> bool {
        self.definition().light
    }

    pub fn is_glass(self) -> bool {
        self.definition().glass
    }

    pub fn is_high_contrast(self) -> bool {
        self.definition().high_contrast
    }

    /// Flat, square corners for classic desktop-player skins.
    pub fn is_square(self) -> bool {
        self.definition().square
    }

    /// Tighter widget spacing for compact skins.
    pub fn is_compact(self) -> bool {
        self.definition().compact
    }

    /// Four corner colours for the translucent backdrop, or `None` for
    /// opaque skins.
    pub fn backdrop(self) -> Option<[Color32; 4]> {
        self.definition().backdrop
    }

    pub fn palette(self) -> Palette {
        self.definition().palette
    }

    /// Which embedded logo to use when no custom icon file is supplied.
    pub fn icon_is_light(self) -> bool {
        self.definition().icon_light
    }

    /// Optional custom logo/window icon path from `skins.json`.
    pub fn icon_path(self) -> Option<PathBuf> {
        self.definition().icon_path
    }
}

#[derive(Debug, Clone)]
struct SkinDefinition {
    id: &'static str,
    label: &'static str,
    aliases: Vec<&'static str>,
    light: bool,
    glass: bool,
    high_contrast: bool,
    square: bool,
    compact: bool,
    palette: Palette,
    backdrop: Option<[Color32; 4]>,
    icon_light: bool,
    icon_path: Option<PathBuf>,
}

impl SkinDefinition {
    fn fallback() -> Self {
        Self {
            id: "wasp",
            label: "MusicWasp Dark",
            aliases: vec!["amber"],
            light: false,
            glass: false,
            high_contrast: false,
            square: false,
            compact: false,
            palette: Palette::default(),
            backdrop: None,
            icon_light: false,
            icon_path: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SkinFile {
    Wrapped { skins: Vec<SkinEntry> },
    List(Vec<SkinEntry>),
}

impl SkinFile {
    fn into_entries(self) -> Vec<SkinEntry> {
        match self {
            SkinFile::Wrapped { skins } => skins,
            SkinFile::List(skins) => skins,
        }
    }
}

#[derive(Deserialize)]
struct SkinEntry {
    id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    light: bool,
    #[serde(default)]
    glass: bool,
    #[serde(default)]
    high_contrast: bool,
    #[serde(default)]
    square: bool,
    #[serde(default)]
    compact: bool,
    #[serde(default)]
    palette: PaletteEntry,
    #[serde(default)]
    backdrop: Option<Vec<String>>,
    #[serde(default)]
    icon: Option<String>,
}

#[derive(Default, Deserialize)]
struct PaletteEntry {
    bg_dark: Option<String>,
    bg_panel: Option<String>,
    bg_row_alt: Option<String>,
    bg_selected: Option<String>,
    accent: Option<String>,
    text_dim: Option<String>,
    text: Option<String>,
    extreme_bg: Option<String>,
    widget_inactive: Option<String>,
    widget_weak: Option<String>,
    widget_hover: Option<String>,
}

fn parse_skin_file(text: &str, base_dir: Option<&Path>) -> Result<Vec<SkinDefinition>, String> {
    let file: SkinFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    file.into_entries()
        .into_iter()
        .map(|entry| parse_skin(entry, base_dir))
        .collect()
}

fn parse_skin(entry: SkinEntry, base_dir: Option<&Path>) -> Result<SkinDefinition, String> {
    let id = leak(entry.id.trim().to_string());
    if id.is_empty() {
        return Err("skin id cannot be empty".to_string());
    }
    let label = leak(
        entry
            .label
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.to_string()),
    );
    let aliases = entry
        .aliases
        .into_iter()
        .filter(|alias| !alias.trim().is_empty())
        .map(leak)
        .collect();
    let fallback = Palette::default();
    let palette = Palette {
        bg_dark: parse_color(entry.palette.bg_dark, fallback.bg_dark)?,
        bg_panel: parse_color(entry.palette.bg_panel, fallback.bg_panel)?,
        bg_row_alt: parse_color(entry.palette.bg_row_alt, fallback.bg_row_alt)?,
        bg_selected: parse_color(entry.palette.bg_selected, fallback.bg_selected)?,
        accent: parse_color(entry.palette.accent, fallback.accent)?,
        text_dim: parse_color(entry.palette.text_dim, fallback.text_dim)?,
        text: parse_color(entry.palette.text, fallback.text)?,
        extreme_bg: parse_color(entry.palette.extreme_bg, fallback.extreme_bg)?,
        widget_inactive: parse_color(entry.palette.widget_inactive, fallback.widget_inactive)?,
        widget_weak: parse_color(entry.palette.widget_weak, fallback.widget_weak)?,
        widget_hover: parse_color(entry.palette.widget_hover, fallback.widget_hover)?,
    };
    let backdrop = match entry.backdrop {
        Some(colors) if colors.len() == 4 => Some([
            parse_required_color(&colors[0])?,
            parse_required_color(&colors[1])?,
            parse_required_color(&colors[2])?,
            parse_required_color(&colors[3])?,
        ]),
        Some(_) => return Err("backdrop must contain four colours".to_string()),
        None => default_backdrop(entry.glass, entry.light),
    };
    let (icon_light, icon_path) = match entry.icon.as_deref() {
        Some("light") => (true, None),
        Some("dark") => (false, None),
        Some(path) if !path.trim().is_empty() => {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else if let Some(base_dir) = base_dir {
                base_dir.join(path)
            } else {
                path
            };
            (entry.light, Some(path))
        }
        _ => (entry.light, None),
    };

    Ok(SkinDefinition {
        id,
        label,
        aliases,
        light: entry.light,
        glass: entry.glass,
        high_contrast: entry.high_contrast,
        square: entry.square,
        compact: entry.compact,
        palette,
        backdrop,
        icon_light,
        icon_path,
    })
}

fn default_backdrop(glass: bool, light: bool) -> Option<[Color32; 4]> {
    if !glass {
        return None;
    }
    Some(if light {
        [
            Color32::from_rgb(236, 244, 255),
            Color32::from_rgb(221, 247, 255),
            Color32::from_rgb(255, 238, 249),
            Color32::from_rgb(247, 251, 255),
        ]
    } else {
        [
            Color32::from_rgb(9, 18, 38),
            Color32::from_rgb(7, 40, 48),
            Color32::from_rgb(31, 14, 52),
            Color32::from_rgb(4, 7, 14),
        ]
    })
}

fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn parse_required_color(value: &str) -> Result<Color32, String> {
    parse_color(Some(value.to_string()), Color32::TRANSPARENT)
}

fn parse_color(value: Option<String>, fallback: Color32) -> Result<Color32, String> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16).map_err(|_| format!("invalid colour {value:?}"))
    };
    match hex.len() {
        6 => Ok(Color32::from_rgb(parse(0..2)?, parse(2..4)?, parse(4..6)?)),
        8 => Ok(Color32::from_rgba_unmultiplied(
            parse(0..2)?,
            parse(2..4)?,
            parse(4..6)?,
            parse(6..8)?,
        )),
        _ => Err(format!("colour {value:?} must be #rrggbb or #rrggbbaa")),
    }
}

fn skins() -> &'static RwLock<Vec<SkinDefinition>> {
    static SKINS: OnceLock<RwLock<Vec<SkinDefinition>>> = OnceLock::new();
    SKINS.get_or_init(|| RwLock::new(embedded_skins()))
}

fn embedded_skins() -> Vec<SkinDefinition> {
    parse_skin_file(include_str!("../../assets/skins.json"), None).unwrap_or_else(|error| {
        eprintln!("MusicWasp: could not parse built-in skins.json: {error}");
        vec![SkinDefinition::fallback()]
    })
}

/// Load `skins.json` from the data directory on top of the built-in list.
/// Entries with an existing id replace it; new ids are appended.
pub fn load_user_skins() {
    let mut definitions = embedded_skins();
    if let Ok(dir) = musicwasp_core::db::data_dir() {
        let path = dir.join("skins.json");
        if path.is_file() {
            match std::fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|text| parse_skin_file(&text, Some(&dir)))
            {
                Ok(user) => {
                    for definition in user {
                        if let Some(existing) = definitions
                            .iter_mut()
                            .find(|existing| existing.id == definition.id)
                        {
                            *existing = definition;
                        } else {
                            definitions.push(definition);
                        }
                    }
                }
                Err(error) => {
                    eprintln!("MusicWasp: ignoring {}: {error}", path.display());
                }
            }
        }
    }
    if !definitions
        .iter()
        .any(|definition| definition.id == Skin::WASP.id)
    {
        definitions.insert(0, SkinDefinition::fallback());
    }
    *skins()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = definitions;
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

impl Default for Palette {
    fn default() -> Self {
        Self {
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
        }
    }
}

static ACTIVE: OnceLock<RwLock<Palette>> = OnceLock::new();

/// Extra UI scale used by the larger-font accessibility mode.
pub const LARGE_TEXT_SCALE: f32 = 1.2;

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
    let glass = skin.is_glass();
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
    v.selection.stroke = Stroke::new(
        if skin.is_high_contrast() {
            2.0
        } else if glass {
            1.5
        } else {
            1.0
        },
        palette.accent,
    );
    v.hyperlink_color = palette.accent;
    v.window_stroke = Stroke::new(
        if skin.is_high_contrast() { 2.0 } else { 1.0 },
        palette.widget_hover,
    );
    if glass {
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 14],
            blur: 34,
            spread: 0,
            color: Color32::from_black_alpha(if skin.is_light() { 55 } else { 130 }),
        };
        v.popup_shadow = egui::epaint::Shadow {
            offset: [0, 10],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if skin.is_light() { 45 } else { 110 }),
        };
        v.window_highlight_topmost = true;
    } else if skin.is_square() {
        v.window_corner_radius = CornerRadius::same(0);
        v.menu_corner_radius = CornerRadius::same(0);
        v.window_shadow = egui::epaint::Shadow {
            offset: [2, 2],
            blur: 0,
            spread: 0,
            color: Color32::from_black_alpha(180),
        };
        v.popup_shadow = egui::epaint::Shadow {
            offset: [2, 2],
            blur: 0,
            spread: 0,
            color: Color32::from_black_alpha(180),
        };
    }
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
        w.corner_radius = CornerRadius::same(if glass {
            8
        } else if skin.is_square() {
            0
        } else {
            3
        });
    }

    ctx.set_theme(if skin.is_light() {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        if glass {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            style.spacing.window_margin = egui::Margin::same(12);
        } else if skin.is_compact() {
            style.spacing.item_spacing = egui::vec2(4.0, 2.0);
            style.spacing.button_padding = egui::vec2(6.0, 2.0);
            style.spacing.window_margin = egui::Margin::same(4);
        } else {
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
        }
        style.spacing.scroll.bar_width = 10.0;
    });
}

/// Apply the accessibility text scale to the whole UI, not just labels, so
/// buttons and table rows grow together and nothing clips.
pub fn apply_text_scale(ctx: &egui::Context, large: bool) {
    ctx.set_zoom_factor(if large { LARGE_TEXT_SCALE } else { 1.0 });
}

/// Paint the colourful gradient behind the translucent glass panels.
pub fn paint_backdrop(ui: &egui::Ui, skin: Skin) {
    let Some(colors) = skin.backdrop() else {
        return;
    };
    let rect = ui.max_rect();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), colors[0]);
    mesh.colored_vertex(rect.right_top(), colors[1]);
    mesh.colored_vertex(rect.right_bottom(), colors[2]);
    mesh.colored_vertex(rect.left_bottom(), colors[3]);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));

    // Soft colour washes imitate the light refraction of liquid glass.
    let (cyan, violet) = if skin.is_light() {
        (
            Color32::from_rgba_unmultiplied(75, 170, 255, 20),
            Color32::from_rgba_unmultiplied(255, 120, 210, 16),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(55, 210, 255, 24),
            Color32::from_rgba_unmultiplied(155, 85, 255, 22),
        )
    };
    let painter = ui.painter();
    painter.circle_filled(
        rect.left_top() + egui::vec2(rect.width() * 0.22, rect.height() * 0.18),
        rect.width().min(rect.height()) * 0.42,
        cyan,
    );
    painter.circle_filled(
        rect.right_bottom() - egui::vec2(rect.width() * 0.18, rect.height() * 0.16),
        rect.width().min(rect.height()) * 0.38,
        violet,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skin_has_a_distinct_palette() {
        let mut seen = Vec::new();
        for skin in Skin::all() {
            let palette = skin.palette();
            assert!(!seen.contains(&palette), "{:?} reuses a palette", skin);
            seen.push(palette);
        }
    }

    #[test]
    fn applying_a_skin_updates_the_shared_palette() {
        let ctx = egui::Context::default();
        apply(&ctx, Skin::MIDNIGHT);
        assert_eq!(accent(), Skin::MIDNIGHT.palette().accent);
        assert_eq!(text(), Skin::MIDNIGHT.palette().text);

        apply(&ctx, Skin::LIGHT);
        assert_eq!(accent(), Skin::LIGHT.palette().accent);
        assert!(Skin::LIGHT.is_light());

        // Leave the default behind for any other test that paints.
        apply(&ctx, Skin::default());
    }

    #[test]
    fn high_contrast_skin_is_dark_and_maximally_legible() {
        let palette = Skin::HIGH_CONTRAST.palette();
        assert!(!Skin::HIGH_CONTRAST.is_light());
        assert_eq!(palette.bg_dark, Color32::BLACK);
        assert_eq!(palette.text, Color32::WHITE);
        assert!(palette.accent.r() > 240);
        assert!(palette.text_dim.r() > 180);
        assert_eq!(Skin::HIGH_CONTRAST.label(), "High contrast");
        assert!(Skin::all().contains(&Skin::HIGH_CONTRAST));
    }

    #[test]
    fn liquid_glass_themes_have_translucent_surfaces_and_backdrops() {
        for skin in [Skin::LIQUID_GLASS_DARK, Skin::LIQUID_GLASS_LIGHT] {
            assert!(skin.is_glass());
            let palette = skin.palette();
            assert!(palette.bg_panel.a() < 255, "{skin:?} panels are opaque");
            assert!(palette.bg_dark.a() < 255, "{skin:?} background is opaque");
            assert!(skin.backdrop().is_some());
            assert!(skin.label().starts_with("Liquid Glass"));
        }
        assert!(Skin::LIQUID_GLASS_DARK.backdrop().is_some());
        assert!(!Skin::WASP.is_glass());
        assert!(Skin::LIQUID_GLASS_LIGHT.is_light());
        assert!(!Skin::LIQUID_GLASS_DARK.is_light());
    }

    #[test]
    fn winamp_classic_skin_is_flat_and_green() {
        let skin = Skin::from_id("winamp_classic");
        assert_eq!(skin.label(), "Winamp Classic");
        assert!(skin.is_square());
        assert!(skin.is_compact());
        assert!(!skin.is_glass());
        let palette = skin.palette();
        assert!(palette.accent.g() > 220);
        assert!(Skin::all().iter().any(|item| item.id() == "winamp_classic"));
    }

    #[test]
    fn external_skin_json_parses_colours_aliases_and_icons() {
        let definitions = parse_skin_file(
            r##"[
                {
                    "id": "custom",
                    "label": "Custom",
                    "aliases": ["c"],
                    "light": true,
                    "glass": true,
                    "high_contrast": false,
                    "palette": {
                        "accent": "#123456",
                        "bg_dark": "#01020380"
                    },
                    "backdrop": ["#000000", "#111111", "#222222", "#333333"],
                    "icon": "logo.png"
                }
            ]"##,
            Some(Path::new("/skins")),
        )
        .unwrap();
        let skin = &definitions[0];
        assert_eq!(skin.id, "custom");
        assert_eq!(skin.label, "Custom");
        assert_eq!(skin.aliases, vec!["c"]);
        assert!(skin.light && skin.glass);
        assert_eq!(skin.palette.accent, Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(skin.palette.bg_dark.a(), 0x80);
        assert_eq!(
            skin.icon_path.as_deref(),
            Some(Path::new("/skins/logo.png"))
        );
        assert_eq!(
            skin.backdrop.unwrap()[1],
            Color32::from_rgb(0x11, 0x11, 0x11)
        );
    }
}
