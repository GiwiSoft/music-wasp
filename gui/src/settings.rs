//! User settings. Currently just rebindable keyboard shortcuts, stored as JSON
//! next to the library database.

use crate::i18n::{tr, Language};
use crate::theme::Skin;
use egui::{Key, Modifiers};
use musicwasp_core::dsp::Equalizer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything the keyboard can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    PlayPause,
    PlaySelected,
    NextTrack,
    PrevTrack,
    SeekForward,
    SeekBackward,
    Stop,
    VolumeUp,
    VolumeDown,
    EditTags,
    Rename,
    FocusSearch,
    OpenSettings,
    ToggleEqualizer,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::PlayPause,
        Action::PlaySelected,
        Action::NextTrack,
        Action::PrevTrack,
        Action::SeekForward,
        Action::SeekBackward,
        Action::Stop,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::EditTags,
        Action::Rename,
        Action::FocusSearch,
        Action::OpenSettings,
        Action::ToggleEqualizer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Play / pause",
            Action::PlaySelected => "Play selected track",
            Action::NextTrack => "Next track",
            Action::PrevTrack => "Previous track",
            Action::SeekForward => "Forward 5 seconds",
            Action::SeekBackward => "Backward 5 seconds",
            Action::Stop => "Stop",
            Action::VolumeUp => "Volume up",
            Action::VolumeDown => "Volume down",
            Action::EditTags => "Edit properties",
            Action::Rename => "Rename field or playlist",
            Action::FocusSearch => "Focus search box",
            Action::OpenSettings => "Open settings",
            Action::ToggleEqualizer => "Open equalizer",
        }
    }

    fn default_token(self) -> &'static str {
        match self {
            Action::PlayPause => "space",
            Action::PlaySelected => "enter",
            Action::NextTrack => "ctrl+right",
            Action::PrevTrack => "ctrl+left",
            Action::SeekForward => "ctrl+shift+right",
            Action::SeekBackward => "ctrl+shift+left",
            Action::Stop => "ctrl+period",
            Action::VolumeUp => "ctrl+up",
            Action::VolumeDown => "ctrl+down",
            Action::EditTags => "ctrl+f2",
            Action::Rename => "f2",
            Action::FocusSearch => "ctrl+f",
            Action::OpenSettings => "ctrl+comma",
            Action::ToggleEqualizer => "ctrl+e",
        }
    }
}

/// A single key plus its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Shortcut {
    pub fn new(key: Key, mods: &Modifiers) -> Self {
        Self {
            key,
            ctrl: mods.ctrl || mods.command,
            alt: mods.alt,
            shift: mods.shift,
        }
    }

    /// Does this shortcut fire this frame?
    pub fn pressed(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.key_pressed(self.key)
                && (i.modifiers.ctrl || i.modifiers.command) == self.ctrl
                && i.modifiers.alt == self.alt
                && i.modifiers.shift == self.shift
        })
    }

    /// Stable text form used in settings.json, e.g. `ctrl+arrowright`.
    pub fn token(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("ctrl+");
        }
        if self.alt {
            out.push_str("alt+");
        }
        if self.shift {
            out.push_str("shift+");
        }
        out.push_str(&self.key.name().to_ascii_lowercase());
        out
    }

    pub fn parse(token: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = None;

        for part in token.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                other => key = find_key(other),
            }
        }

        Some(Self {
            key: key?,
            ctrl,
            alt,
            shift,
        })
    }

    /// Human readable form for the settings panel.
    pub fn label(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        out.push_str(pretty_key(self.key));
        out
    }
}

/// egui's names are capitalised (`Right`, `Space`); settings files are lower case.
fn find_key(name: &str) -> Option<Key> {
    Key::ALL.iter().copied().find(|k| {
        k.name().eq_ignore_ascii_case(name) || k.symbol_or_name().eq_ignore_ascii_case(name)
    })
}

fn default_shortcut(action: Action) -> Option<Shortcut> {
    Shortcut::parse(action.default_token())
}

fn pretty_key(key: Key) -> &'static str {
    match key {
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Period => ".",
        Key::Comma => ",",
        other => other.name(),
    }
}

/// Which browser sections are folded shut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct FoldedSections {
    pub genre: bool,
    pub artist: bool,
    pub album: bool,
    pub playlists: bool,
}

/// A track-list column. The order here is also the default order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackColumn {
    Number,
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Duration,
    Rating,
    Format,
}

impl TrackColumn {
    pub const ALL: [TrackColumn; 9] = [
        TrackColumn::Number,
        TrackColumn::Title,
        TrackColumn::Artist,
        TrackColumn::Album,
        TrackColumn::Genre,
        TrackColumn::Year,
        TrackColumn::Duration,
        TrackColumn::Rating,
        TrackColumn::Format,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackColumn::Number => "#",
            TrackColumn::Title => tr("column.title"),
            TrackColumn::Artist => tr("column.artist"),
            TrackColumn::Album => tr("column.album"),
            TrackColumn::Genre => tr("column.genre"),
            TrackColumn::Year => tr("column.year"),
            TrackColumn::Duration => tr("column.time"),
            TrackColumn::Rating => tr("column.rating"),
            TrackColumn::Format => tr("column.format"),
        }
    }

    pub fn default_width(self) -> f32 {
        match self {
            TrackColumn::Number => 44.0,
            TrackColumn::Title => 220.0,
            TrackColumn::Artist => 150.0,
            TrackColumn::Album => 170.0,
            TrackColumn::Genre => 110.0,
            TrackColumn::Year => 52.0,
            TrackColumn::Duration => 52.0,
            TrackColumn::Rating => 90.0,
            TrackColumn::Format => 78.0,
        }
    }

    pub fn min_width(self) -> f32 {
        match self {
            TrackColumn::Number => 36.0,
            TrackColumn::Title => 120.0,
            TrackColumn::Artist => 70.0,
            TrackColumn::Album => 70.0,
            TrackColumn::Genre => 60.0,
            TrackColumn::Year => 36.0,
            TrackColumn::Duration => 36.0,
            TrackColumn::Rating => 70.0,
            TrackColumn::Format => 50.0,
        }
    }
}

/// How dense the track table is, and whether it includes album artwork.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackLayout {
    #[default]
    Standard,
    Compact,
    AlbumArt,
}

impl TrackLayout {
    pub const ALL: [TrackLayout; 3] = [
        TrackLayout::Standard,
        TrackLayout::Compact,
        TrackLayout::AlbumArt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackLayout::Standard => tr("ui.standard"),
            TrackLayout::Compact => tr("ui.compact"),
            TrackLayout::AlbumArt => tr("ui.album_art"),
        }
    }
}

/// Persisted track-list columns: order, visibility, widths and layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnConfig {
    pub order: Vec<TrackColumn>,
    pub hidden: Vec<TrackColumn>,
    pub widths: BTreeMap<TrackColumn, f32>,
    pub layout: TrackLayout,
}

impl Default for ColumnConfig {
    fn default() -> Self {
        Self {
            order: TrackColumn::ALL.to_vec(),
            hidden: Vec::new(),
            widths: BTreeMap::new(),
            layout: TrackLayout::Standard,
        }
    }
}

impl ColumnConfig {
    /// Every column exactly once, with new columns appended to old settings.
    pub fn normalized_order(&self) -> Vec<TrackColumn> {
        let mut order = Vec::with_capacity(TrackColumn::ALL.len());
        for column in &self.order {
            if !order.contains(column) {
                order.push(*column);
            }
        }
        for column in TrackColumn::ALL {
            if !order.contains(&column) {
                order.push(column);
            }
        }
        order
    }

    pub fn is_hidden(&self, column: TrackColumn) -> bool {
        self.hidden.contains(&column)
    }

    /// Columns in display order, never empty.
    pub fn visible(&self) -> Vec<TrackColumn> {
        let mut visible: Vec<TrackColumn> = self
            .normalized_order()
            .into_iter()
            .filter(|column| !self.is_hidden(*column))
            .collect();
        if visible.is_empty() {
            visible.push(TrackColumn::Title);
        }
        visible
    }

    pub fn width(&self, column: TrackColumn) -> f32 {
        self.widths
            .get(&column)
            .copied()
            .filter(|width| width.is_finite() && *width > 0.0)
            .unwrap_or_else(|| column.default_width())
            .max(column.min_width())
    }

    pub fn set_width(&mut self, column: TrackColumn, width: f32) {
        if width.is_finite() && width > 0.0 {
            self.widths.insert(column, width.max(column.min_width()));
        }
    }

    /// Move a visible column one slot earlier or later in the full order.
    pub fn move_column(&mut self, column: TrackColumn, delta: i32) {
        let order = self.normalized_order();
        let Some(index) = order.iter().position(|item| *item == column) else {
            return;
        };
        let target = index as i32 + delta;
        if target < 0 || target as usize >= order.len() {
            return;
        }
        let target = target as usize;
        let mut order = order;
        order.swap(index, target);
        self.order = order;
    }

    pub fn reset(&mut self) {
        let layout = self.layout;
        *self = Self::default();
        self.layout = layout;
    }
}

/// How the visualizer chooses the next preset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerMode {
    /// Stay on the selected preset.
    #[default]
    Selected,
    /// Rotate through every preset in a random order.
    Shuffle,
}

/// Visualizer preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualizerSettings {
    pub enabled: bool,
    pub mode: VisualizerMode,
    /// Preset name to restore at startup.
    pub preset: String,
    /// Seconds between automatic changes in shuffle mode.
    pub interval_secs: u32,
    /// Target frame rate: 30 or 60.
    pub fps: u32,
}

impl Default for VisualizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: VisualizerMode::Selected,
            preset: String::new(),
            interval_secs: 45,
            fps: 30,
        }
    }
}

/// Persisted state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// action -> shortcut token. Missing entries fall back to the default.
    #[serde(default)]
    pub shortcuts: BTreeMap<Action, String>,
    /// Equalizer settings, shared with the audio engine.
    #[serde(default)]
    pub equalizer: Equalizer,
    /// Colour skin.
    #[serde(default)]
    pub skin: Skin,
    /// UI language. English is the source language and default.
    #[serde(default)]
    pub language: Language,
    /// Scale the whole interface for larger text and controls.
    #[serde(default)]
    pub large_fonts: bool,
    /// Collapsed state of the left browser sections.
    #[serde(default)]
    pub folded: FoldedSections,
    /// Track-list column order, visibility, widths and layout.
    #[serde(default)]
    pub columns: ColumnConfig,
    /// Show the play-queue panel on the right.
    #[serde(default = "default_queue_open")]
    pub queue_open: bool,
    /// Show the lyrics panel on the right.
    #[serde(default)]
    pub lyrics_open: bool,
    /// Watch scanned folders and import new or changed audio automatically.
    #[serde(default = "default_watch_enabled")]
    pub watch_enabled: bool,
    /// Apply ReplayGain tags to playback.
    #[serde(default = "default_replaygain_enabled")]
    pub replaygain: bool,
    /// Prefer album gain over track gain.
    #[serde(default)]
    pub replaygain_album: bool,
    /// Selected output device name; `None` uses the system default.
    #[serde(default)]
    pub output_device: Option<String>,
    /// Overlap length between tracks in seconds; 0 keeps gapless joining.
    #[serde(default)]
    pub crossfade_secs: f32,
    /// Submit listens to ListenBrainz.
    #[serde(default)]
    pub scrobble_listenbrainz: bool,
    /// ListenBrainz user token. Empty means scrobbling is off.
    #[serde(default)]
    pub listenbrainz_token: String,
    /// Folders handed to the OS watcher. Added by a successful scan.
    #[serde(default)]
    pub watch_folders: Vec<String>,
    /// Show the compact always-on-top player window.
    #[serde(default)]
    pub mini_player: bool,
    /// Show the Linux tray icon.
    #[serde(default)]
    pub tray_enabled: bool,
    /// MilkDrop visualizer preferences.
    #[serde(default)]
    pub visualizer: VisualizerSettings,
    /// Template used by the file organizer.
    #[serde(default = "default_organize_pattern")]
    pub organize_pattern: String,
    /// Destination root for the organizer; empty uses the first scan folder.
    #[serde(default)]
    pub organize_root: String,
    /// What the app looked and sounded like when it last closed.
    #[serde(default)]
    pub session: Session,
}

fn default_watch_enabled() -> bool {
    true
}

fn default_replaygain_enabled() -> bool {
    true
}

fn default_organize_pattern() -> String {
    musicwasp_core::organize::DEFAULT_PATTERN.to_string()
}

/// The view that was open when the app last closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionView {
    #[default]
    Library,
    Playlist(i64),
    Smart(i64),
    Podcasts,
    Radio,
}

/// Window size and, where the platform reports it, screen position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
}

/// Restorable session state. Missing fields fall back to the defaults so old
/// `settings.json` files keep working.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub view: SessionView,
    pub search: String,
    pub selected: Vec<i64>,
    /// `None` means the app has not saved a volume yet; use the built-in 0.8.
    pub volume: Option<f32>,
    pub queue: Vec<i64>,
    pub queue_pos: Option<usize>,
    pub window: Option<WindowGeometry>,
}

fn default_queue_open() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        let mut shortcuts = BTreeMap::new();
        for action in Action::ALL {
            shortcuts.insert(action, action.default_token().to_string());
        }
        Self {
            shortcuts,
            equalizer: Equalizer::default(),
            skin: Skin::default(),
            language: Language::default(),
            large_fonts: false,
            folded: FoldedSections::default(),
            columns: ColumnConfig::default(),
            queue_open: default_queue_open(),
            lyrics_open: false,
            watch_enabled: default_watch_enabled(),
            replaygain: default_replaygain_enabled(),
            replaygain_album: false,
            output_device: None,
            crossfade_secs: 0.0,
            scrobble_listenbrainz: false,
            listenbrainz_token: String::new(),
            watch_folders: Vec::new(),
            mini_player: false,
            tray_enabled: false,
            visualizer: VisualizerSettings::default(),
            organize_pattern: default_organize_pattern(),
            organize_root: String::new(),
            session: Session::default(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        musicwasp_core::db::data_dir()
            .map(|d| d.join("settings.json"))
            .unwrap_or_else(|_| PathBuf::from("settings.json"))
    }

    pub fn load(path: &Path) -> Self {
        let mut settings = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("ignoring invalid settings at {}: {e}", path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        settings.migrate();
        settings
    }

    /// Keep settings written by older versions usable.
    ///
    /// F2 used to open the tag editor. It now starts a rename, so move any
    /// stored tag-editor binding out of the way instead of creating a conflict.
    fn migrate(&mut self) {
        let rename_missing = !self.shortcuts.contains_key(&Action::Rename);
        let tags_on_f2 = self.shortcuts.get(&Action::EditTags).map(String::as_str) == Some("f2");
        if rename_missing && tags_on_f2 {
            self.shortcuts
                .insert(Action::EditTags, "ctrl+f2".to_string());
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Current binding for `action`.
    ///
    /// `None` means the action is deliberately unbound. Missing or unreadable
    /// entries fall back to the action's default.
    pub fn shortcut(&self, action: Action) -> Option<Shortcut> {
        match self.shortcuts.get(&action) {
            Some(token) if token.is_empty() => None,
            Some(token) => Shortcut::parse(token).or_else(|| default_shortcut(action)),
            None => default_shortcut(action),
        }
    }

    pub fn set(&mut self, action: Action, shortcut: Shortcut) {
        self.shortcuts.insert(action, shortcut.token());
    }

    /// Unbind, leaving the action keyboard-less.
    pub fn unbind(&mut self, action: Action) {
        // An explicit empty string means "bound to nothing"; absence would fall
        // back to the default.
        self.shortcuts.insert(action, String::new());
    }

    /// Actions sharing a binding with another action.
    pub fn conflicts(&self) -> Vec<Action> {
        let mut seen: BTreeMap<String, Vec<Action>> = BTreeMap::new();
        for action in Action::ALL {
            if let Some(shortcut) = self.shortcut(action) {
                seen.entry(shortcut.token()).or_default().push(action);
            }
        }
        let mut out: Vec<Action> = seen
            .into_values()
            .filter(|group| group.len() > 1)
            .flatten()
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
#[path = "../tests/unit/settings.rs"]
mod tests;
