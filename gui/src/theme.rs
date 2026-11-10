//! MusicWasp skins.
//!
//! A skin is a named palette applied to egui. Colours are read through
//! [`accent`] and friends so widgets that paint themselves (album art
//! placeholders, drag markers, equalizer curve) follow the active skin too.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
            .find(|definition| definition.id == id || definition.aliases.contains(&id))
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

    /// Optional background image, typically extracted from a Winamp skin.
    pub fn background_path(self) -> Option<PathBuf> {
        self.definition().background
    }

    /// Optional linear gradient stops for the window background.
    pub fn background_gradient(self) -> Option<Vec<Color32>> {
        self.definition().background_gradient
    }

    /// Gradient direction in degrees: 0 = left-to-right, 90 = top-to-bottom.
    pub fn background_angle(self) -> f32 {
        self.definition().background_angle
    }

    pub fn has_background(self) -> bool {
        self.background_path().is_some() || self.background_gradient().is_some()
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
    background: Option<PathBuf>,
    background_gradient: Option<Vec<Color32>>,
    background_angle: f32,
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
            background: None,
            background_gradient: None,
            background_angle: 135.0,
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
    background: Option<String>,
    #[serde(default)]
    background_gradient: Option<Vec<String>>,
    #[serde(default)]
    background_angle: Option<f32>,
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
    let background_gradient = match entry.background_gradient {
        Some(colors) => {
            if colors.len() < 2 {
                return Err("background_gradient needs at least two colours".to_string());
            }
            Some(
                colors
                    .iter()
                    .map(|color| parse_required_color(color))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }
        None => None,
    };
    let background_angle = entry.background_angle.unwrap_or(135.0);
    let background = entry.background.as_deref().and_then(|path| {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let path = PathBuf::from(path);
        Some(if path.is_absolute() {
            path
        } else if let Some(base_dir) = base_dir {
            base_dir.join(path)
        } else {
            path
        })
    });
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
        background,
        background_gradient,
        background_angle,
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

/// One editable colour slot, shared by the editor and `skins.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteField {
    BgDark,
    BgPanel,
    BgRowAlt,
    BgSelected,
    Accent,
    TextDim,
    Text,
    ExtremeBg,
    WidgetInactive,
    WidgetWeak,
    WidgetHover,
}

impl PaletteField {
    pub const ALL: [PaletteField; 11] = [
        PaletteField::BgDark,
        PaletteField::BgPanel,
        PaletteField::BgRowAlt,
        PaletteField::BgSelected,
        PaletteField::Accent,
        PaletteField::TextDim,
        PaletteField::Text,
        PaletteField::ExtremeBg,
        PaletteField::WidgetInactive,
        PaletteField::WidgetWeak,
        PaletteField::WidgetHover,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaletteField::BgDark => "Window background",
            PaletteField::BgPanel => "Panel background",
            PaletteField::BgRowAlt => "Alternate row",
            PaletteField::BgSelected => "Selected row",
            PaletteField::Accent => "Accent",
            PaletteField::TextDim => "Dimmed text",
            PaletteField::Text => "Text",
            PaletteField::ExtremeBg => "Extreme background",
            PaletteField::WidgetInactive => "Widget",
            PaletteField::WidgetWeak => "Widget (weak)",
            PaletteField::WidgetHover => "Widget hover",
        }
    }

    fn key(self) -> &'static str {
        match self {
            PaletteField::BgDark => "bg_dark",
            PaletteField::BgPanel => "bg_panel",
            PaletteField::BgRowAlt => "bg_row_alt",
            PaletteField::BgSelected => "bg_selected",
            PaletteField::Accent => "accent",
            PaletteField::TextDim => "text_dim",
            PaletteField::Text => "text",
            PaletteField::ExtremeBg => "extreme_bg",
            PaletteField::WidgetInactive => "widget_inactive",
            PaletteField::WidgetWeak => "widget_weak",
            PaletteField::WidgetHover => "widget_hover",
        }
    }

    pub fn get(self, palette: &Palette) -> Color32 {
        match self {
            PaletteField::BgDark => palette.bg_dark,
            PaletteField::BgPanel => palette.bg_panel,
            PaletteField::BgRowAlt => palette.bg_row_alt,
            PaletteField::BgSelected => palette.bg_selected,
            PaletteField::Accent => palette.accent,
            PaletteField::TextDim => palette.text_dim,
            PaletteField::Text => palette.text,
            PaletteField::ExtremeBg => palette.extreme_bg,
            PaletteField::WidgetInactive => palette.widget_inactive,
            PaletteField::WidgetWeak => palette.widget_weak,
            PaletteField::WidgetHover => palette.widget_hover,
        }
    }

    pub fn set(self, palette: &mut Palette, color: Color32) {
        match self {
            PaletteField::BgDark => palette.bg_dark = color,
            PaletteField::BgPanel => palette.bg_panel = color,
            PaletteField::BgRowAlt => palette.bg_row_alt = color,
            PaletteField::BgSelected => palette.bg_selected = color,
            PaletteField::Accent => palette.accent = color,
            PaletteField::TextDim => palette.text_dim = color,
            PaletteField::Text => palette.text = color,
            PaletteField::ExtremeBg => palette.extreme_bg = color,
            PaletteField::WidgetInactive => palette.widget_inactive = color,
            PaletteField::WidgetWeak => palette.widget_weak = color,
            PaletteField::WidgetHover => palette.widget_hover = color,
        }
    }
}

/// Data directory, created when missing. `skins.json` lives here.
pub fn skins_folder() -> Result<PathBuf, String> {
    let dir = musicwasp_core::db::data_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Save `palette` as an override of `skin` in `skins.json`, then reload.
pub fn save_palette(skin: Skin, palette: Palette) -> Result<Skin, String> {
    let definition = skin.definition();
    let entry = entry_value(&definition, palette, true);
    upsert_entry(&entry, definition.id)?;
    load_user_skins();
    Ok(Skin::from_id(skin.id()))
}

/// Copy `skin` into `skins.json` under a unique id.
pub fn duplicate(skin: Skin) -> Result<Skin, String> {
    let definition = skin.definition();
    let (id, label) = unique_copy(&definition);
    let mut entry = entry_value(&definition, definition.palette, false);
    entry["id"] = serde_json::Value::String(id.clone());
    entry["label"] = serde_json::Value::String(label);
    upsert_entry(&entry, &id)?;
    load_user_skins();
    Ok(Skin::from_id(&id))
}

fn unique_copy(definition: &SkinDefinition) -> (String, String) {
    let base = format!("{}-copy", definition.id);
    let mut id = base.clone();
    let mut index = 2;
    while Skin::all().iter().any(|skin| skin.id() == id) {
        id = format!("{base}{index}");
        index += 1;
    }
    let mut label = format!("{} copy", definition.label);
    if index > 2 {
        label.push_str(&format!(" {}", index - 1));
    }
    (id, label)
}

fn palette_json(palette: Palette) -> serde_json::Map<String, serde_json::Value> {
    PaletteField::ALL
        .iter()
        .map(|field| {
            (
                field.key().to_string(),
                serde_json::Value::String(hex(field.get(&palette))),
            )
        })
        .collect()
}

fn entry_value(
    definition: &SkinDefinition,
    palette: Palette,
    include_aliases: bool,
) -> serde_json::Value {
    let colors = palette_json(palette);
    let mut entry = serde_json::json!({
        "id": definition.id,
        "label": definition.label,
        "light": definition.light,
        "glass": definition.glass,
        "high_contrast": definition.high_contrast,
        "square": definition.square,
        "compact": definition.compact,
        "palette": colors,
    });
    if include_aliases && !definition.aliases.is_empty() {
        entry["aliases"] = serde_json::json!(definition.aliases);
    }
    if let Some(backdrop) = definition.backdrop {
        entry["backdrop"] =
            serde_json::json!(backdrop.iter().map(|color| hex(*color)).collect::<Vec<_>>());
    }
    if let Some(path) = &definition.background {
        entry["background"] = serde_json::Value::String(path.to_string_lossy().to_string());
    }
    if let Some(colors) = &definition.background_gradient {
        entry["background_gradient"] =
            serde_json::json!(colors.iter().map(|color| hex(*color)).collect::<Vec<_>>());
        entry["background_angle"] = serde_json::json!(definition.background_angle);
    }
    if let Some(path) = &definition.icon_path {
        entry["icon"] = serde_json::Value::String(path.to_string_lossy().to_string());
    } else {
        entry["icon"] = serde_json::Value::String(
            if definition.icon_light {
                "light"
            } else {
                "dark"
            }
            .to_string(),
        );
    }
    entry
}

fn upsert_entry(entry: &serde_json::Value, id: &str) -> Result<(), String> {
    let path = skins_folder()?.join("skins.json");
    let mut entries = user_entries(&path)?;
    match entries
        .iter_mut()
        .find(|item| item.get("id").and_then(|value| value.as_str()) == Some(id))
    {
        Some(existing) => *existing = entry.clone(),
        None => entries.push(entry.clone()),
    }
    let document = serde_json::json!({ "skins": entries });
    let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())?;
    Ok(())
}

fn user_entries(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("invalid skins.json: {error}"))?;
    match value {
        serde_json::Value::Array(entries) => Ok(entries),
        serde_json::Value::Object(mut map) => match map.remove("skins") {
            Some(serde_json::Value::Array(entries)) => Ok(entries),
            _ => Err("skins.json must hold a list or an object with \"skins\"".to_string()),
        },
        _ => Err("skins.json must hold a list or an object with \"skins\"".to_string()),
    }
}

fn hex(color: Color32) -> String {
    if color.a() == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r(),
            color.g(),
            color.b(),
            color.a()
        )
    }
}

static ACTIVE: OnceLock<RwLock<Palette>> = OnceLock::new();
static ACTIVE_SKIN: OnceLock<RwLock<Skin>> = OnceLock::new();
static BACKGROUNDS: OnceLock<RwLock<HashMap<PathBuf, (u128, egui::TextureHandle)>>> =
    OnceLock::new();

/// Extra UI scale used by the larger-font accessibility mode.
pub const LARGE_TEXT_SCALE: f32 = 1.2;

/// Fixed-size pill drawn over the seek rail. Keeping this independent of the
/// slider height stops the cursor from changing size between skins or layouts.
pub const SEEK_HANDLE_WIDTH: f32 = 14.0;
pub const SEEK_HANDLE_HEIGHT: f32 = 20.0;

/// Named font family holding the semibold face. Register it in
/// [`crate::icons::install`]; use [`semibold`] instead of touching it directly.
pub const SEMIBOLD_FAMILY: &str = "semibold";

fn active() -> &'static RwLock<Palette> {
    ACTIVE.get_or_init(|| RwLock::new(Skin::default().palette()))
}

fn active_skin() -> &'static RwLock<Skin> {
    ACTIVE_SKIN.get_or_init(|| RwLock::new(Skin::default()))
}

/// Palette currently painted by the UI.
pub fn current() -> Palette {
    *active()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Skin currently installed.
pub fn skin() -> Skin {
    *active_skin()
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
pub fn bg_dark() -> Color32 {
    current().bg_dark
}
pub fn bg_panel() -> Color32 {
    current().bg_panel
}
pub fn bg_row_alt() -> Color32 {
    current().bg_row_alt
}
pub fn bg_selected() -> Color32 {
    current().bg_selected
}

pub fn is_compact() -> bool {
    skin().is_compact()
}

pub fn is_square() -> bool {
    skin().is_square()
}

/// Rounded corner for ordinary surfaces, flattened for the classic square
/// skins and tightened for compact ones.
pub fn corner(radius: u8) -> CornerRadius {
    if is_square() {
        CornerRadius::ZERO
    } else if is_compact() {
        CornerRadius::same(radius.saturating_sub(2).max(1))
    } else {
        CornerRadius::same(radius)
    }
}

/// Fully rounded corners, for pills and circular controls.
pub fn pill() -> CornerRadius {
    CornerRadius::same(255)
}

/// Blend `a` towards `b`; `t = 0` keeps `a`, `t = 1` yields `b`.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

/// `color` at a new opacity.
pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// Hover wash for rows and panels: a faint step up from the background.
pub fn hover_fill() -> Color32 {
    let palette = current();
    if skin().is_high_contrast() {
        palette.widget_hover
    } else {
        blend(palette.widget_hover, palette.bg_dark, 0.62)
    }
}

/// Hairline used for dividers and panel borders.
pub fn hairline() -> Color32 {
    let palette = current();
    if skin().is_high_contrast() {
        palette.widget_hover
    } else {
        blend(palette.widget_weak, palette.bg_dark, 0.35)
    }
}

/// A raised surface colour (chips, search fields).
pub fn raised() -> Color32 {
    let palette = current();
    if skin().is_high_contrast() {
        palette.widget_inactive
    } else {
        blend(palette.widget_inactive, palette.bg_panel, 0.55)
    }
}

/// Semibold text in the bundled display face.
pub fn semibold(text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text)
        .family(egui::FontFamily::Name(SEMIBOLD_FAMILY.into()))
        .size(size)
}

/// Install `skin` and rebuild the egui visuals.
pub fn apply(ctx: &egui::Context, skin: Skin) {
    apply_palette(ctx, skin, skin.palette());
}

/// Apply an edited palette immediately, keeping the skin's other traits.
pub fn preview(ctx: &egui::Context, skin: Skin, palette: Palette) {
    apply_palette(ctx, skin, palette);
}

fn apply_palette(ctx: &egui::Context, skin: Skin, palette: Palette) {
    let glass = skin.is_glass();
    let square = skin.is_square();
    let compact = skin.is_compact();
    let high_contrast = skin.is_high_contrast();
    let palette = if skin.has_background() {
        with_background_alpha(palette)
    } else {
        palette
    };
    *active()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = palette;
    *active_skin()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = skin;

    let mut v = if skin.is_light() {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    let radius = if square {
        0
    } else if compact {
        2
    } else if glass {
        9
    } else {
        6
    };
    let surface_radius = if square {
        0
    } else if compact {
        4
    } else if glass {
        14
    } else {
        10
    };
    let hairline = if high_contrast {
        palette.widget_hover
    } else {
        blend(palette.widget_weak, palette.bg_dark, 0.35)
    };

    v.override_text_color = Some(palette.text);
    v.panel_fill = palette.bg_dark;
    v.window_fill = palette.bg_panel;
    v.extreme_bg_color = if high_contrast {
        palette.extreme_bg
    } else {
        blend(palette.extreme_bg, palette.bg_panel, 0.25)
    };
    v.faint_bg_color = if high_contrast {
        palette.bg_row_alt
    } else {
        blend(palette.bg_row_alt, palette.bg_dark, 0.55)
    };
    v.selection.bg_fill = if high_contrast || palette.bg_selected.a() < 255 {
        palette.bg_selected
    } else {
        blend(palette.bg_selected, palette.bg_dark, 0.26)
    };
    // A zero-width stroke still supplies the accent text colour that egui
    // uses for selected labels; it simply does not paint an outline.
    v.selection.stroke = Stroke::new(if high_contrast { 2.0 } else { 0.0 }, palette.accent);
    v.hyperlink_color = palette.accent;
    v.window_stroke = Stroke::new(1.0, hairline);
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.window_corner_radius = CornerRadius::same(surface_radius);
    v.menu_corner_radius = CornerRadius::same(surface_radius.saturating_sub(2));
    if square {
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
    } else {
        let window_alpha = if skin.is_light() { 26 } else { 90 };
        let popup_alpha = if skin.is_light() { 22 } else { 80 };
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, if glass { 14 } else { 8 }],
            blur: if glass { 34 } else { 22 },
            spread: 0,
            color: Color32::from_black_alpha(window_alpha),
        };
        v.popup_shadow = egui::epaint::Shadow {
            offset: [0, if glass { 10 } else { 6 }],
            blur: if glass { 24 } else { 16 },
            spread: 0,
            color: Color32::from_black_alpha(popup_alpha),
        };
        v.window_highlight_topmost = glass;
    }

    v.widgets.noninteractive.bg_fill = palette.bg_panel;
    v.widgets.noninteractive.weak_bg_fill = palette.bg_panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hairline);
    v.widgets.inactive.bg_fill = palette.widget_inactive;
    v.widgets.inactive.weak_bg_fill = palette.widget_weak;
    v.widgets.hovered.bg_fill = if high_contrast {
        palette.widget_hover
    } else {
        blend(palette.widget_hover, palette.bg_dark, 0.62)
    };
    v.widgets.hovered.weak_bg_fill = palette.widget_hover;
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
        w.corner_radius = CornerRadius::same(radius);
        if !high_contrast {
            w.bg_stroke = Stroke::NONE;
        }
    }
    v.widgets.noninteractive.corner_radius = CornerRadius::same(radius);

    ctx.set_theme(if skin.is_light() {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        style.animation_time = 0.15;
        if glass {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
            style.spacing.window_margin = egui::Margin::same(14);
            style.spacing.menu_margin = egui::Margin::same(8);
        } else if compact || square {
            style.spacing.item_spacing = egui::vec2(4.0, 2.0);
            style.spacing.button_padding = egui::vec2(6.0, 2.0);
            style.spacing.window_margin = egui::Margin::same(6);
            style.spacing.menu_margin = egui::Margin::same(4);
        } else {
            style.spacing.item_spacing = egui::vec2(7.0, 5.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.window_margin = egui::Margin::same(12);
            style.spacing.menu_margin = egui::Margin::same(7);
        }
        style.spacing.interact_size.y = if compact || square { 18.0 } else { 24.0 };
        style.spacing.scroll.bar_width = if compact { 8.0 } else { 10.0 };
        if !compact && !square && !high_contrast {
            // Overlay scrollbars keep the chrome quiet until the pointer
            // enters a scrollable area.
            let mut scroll = egui::style::ScrollStyle::floating();
            scroll.floating_width = 5.0;
            scroll.floating_allocated_width = 14.0;
            scroll.bar_inner_margin = 2.0;
            style.spacing.scroll = scroll;
        }
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(13.5, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.0, egui::FontFamily::Monospace),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(20.0, egui::FontFamily::Name(SEMIBOLD_FAMILY.into())),
        );
    });
}

/// Apply the accessibility text scale to the whole UI, not just labels, so
/// buttons and table rows grow together and nothing clips.
pub fn apply_text_scale(ctx: &egui::Context, large: bool) {
    ctx.set_zoom_factor(if large { LARGE_TEXT_SCALE } else { 1.0 });
}

/// Paint the colourful gradient behind the translucent glass panels.
pub fn paint_backdrop(ui: &egui::Ui, skin: Skin) {
    let rect = ui.max_rect();
    let gradient = skin.background_gradient();
    if let Some(colors) = &gradient {
        paint_linear_gradient(ui.painter(), rect, colors, skin.background_angle());
    }
    if let Some(path) = skin.background_path() {
        if let Some(texture) = background_texture(ui.ctx(), &path) {
            let size = texture.size_vec2();
            if size.x > 0.0 && size.y > 0.0 {
                let scale = (rect.width() / size.x).max(rect.height() / size.y);
                let drawn = egui::Rect::from_center_size(rect.center(), size * scale);
                ui.painter().image(
                    texture.id(),
                    drawn,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let overlay = if skin.is_light() {
                    Color32::from_white_alpha(60)
                } else {
                    Color32::from_black_alpha(95)
                };
                ui.painter().rect_filled(rect, 0.0, overlay);
                return;
            }
        }
    }
    if gradient.is_some() {
        let overlay = if skin.is_light() {
            Color32::from_white_alpha(45)
        } else {
            Color32::from_black_alpha(70)
        };
        ui.painter().rect_filled(rect, 0.0, overlay);
        return;
    }

    let Some(colors) = skin.backdrop() else {
        return;
    };
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

/// Surfaces become partially transparent when a wallpaper is set, so the
/// image remains visible behind panels without hurting text contrast.
fn with_background_alpha(palette: Palette) -> Palette {
    let fade = |color: Color32, max_alpha: u8| {
        let [r, g, b, alpha] = color.to_srgba_unmultiplied();
        Color32::from_rgba_unmultiplied(r, g, b, alpha.min(max_alpha))
    };
    Palette {
        bg_dark: fade(palette.bg_dark, 205),
        bg_panel: fade(palette.bg_panel, 195),
        bg_row_alt: fade(palette.bg_row_alt, 175),
        bg_selected: fade(palette.bg_selected, 220),
        extreme_bg: fade(palette.extreme_bg, 215),
        widget_inactive: fade(palette.widget_inactive, 205),
        widget_weak: fade(palette.widget_weak, 200),
        widget_hover: fade(palette.widget_hover, 215),
        ..palette
    }
}

fn paint_linear_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    colors: &[Color32],
    angle_degrees: f32,
) {
    let angle = angle_degrees.to_radians();
    let direction = egui::vec2(angle.cos(), angle.sin());
    let perpendicular = egui::vec2(-direction.y, direction.x);
    let half_span =
        rect.width().abs() * direction.x.abs() + rect.height().abs() * direction.y.abs();
    let reach = rect.size().length();
    let center = rect.center();
    let start = center - direction * (half_span * 0.5);

    const BANDS: usize = 64;
    for band in 0..BANDS {
        let t0 = band as f32 / BANDS as f32;
        let t1 = (band + 1) as f32 / BANDS as f32;
        let color = gradient_color(colors, (t0 + t1) * 0.5);
        let p0 = start + direction * (half_span * t0);
        let p1 = start + direction * (half_span * t1);
        let points = vec![
            p0 - perpendicular * reach,
            p0 + perpendicular * reach,
            p1 + perpendicular * reach,
            p1 - perpendicular * reach,
        ];
        painter.add(egui::Shape::convex_polygon(
            points,
            color,
            egui::Stroke::NONE,
        ));
    }
}

fn gradient_color(colors: &[Color32], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    if colors.len() == 1 {
        return colors[0];
    }
    let scaled = t * (colors.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(colors.len() - 2);
    let local = scaled - index as f32;
    let a = colors[index];
    let b = colors[index + 1];
    Color32::from_rgb(
        (a.r() as f32 * (1.0 - local) + b.r() as f32 * local).round() as u8,
        (a.g() as f32 * (1.0 - local) + b.g() as f32 * local).round() as u8,
        (a.b() as f32 * (1.0 - local) + b.b() as f32 * local).round() as u8,
    )
}

fn background_texture(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let meta = std::fs::metadata(path).ok()?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
        ^ (meta.len() as u128).rotate_left(17);
    let cache = BACKGROUNDS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some((cached_stamp, texture)) = cache
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(path)
    {
        if *cached_stamp == stamp {
            return Some(texture.clone());
        }
    }
    let image = crate::images::open(path).ok()?.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let texture = ctx.load_texture(
        format!("skin-background-{}", path.display()),
        color,
        egui::TextureOptions::LINEAR,
    );
    cache
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), (stamp, texture.clone()));
    Some(texture)
}

#[cfg(test)]
#[path = "../tests/unit/theme.rs"]
mod tests;
