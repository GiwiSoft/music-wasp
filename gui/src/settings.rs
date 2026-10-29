//! User settings. Currently just rebindable keyboard shortcuts, stored as JSON
//! next to the library database.

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
            Action::EditTags => "Edit tags",
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
    /// Collapsed state of the left browser sections.
    #[serde(default)]
    pub folded: FoldedSections,
    /// Show the play-queue panel on the right.
    #[serde(default = "default_queue_open")]
    pub queue_open: bool,
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
            folded: FoldedSections::default(),
            queue_open: default_queue_open(),
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

    pub fn reset_defaults(&mut self) {
        *self = Settings::default();
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
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for token in [
            "space",
            "enter",
            "f2",
            "ctrl+right",
            "ctrl+shift+space",
            "ctrl+shift+right",
            "ctrl+shift+left",
            "alt+period",
        ] {
            let parsed = Shortcut::parse(token).expect(token);
            assert_eq!(parsed.token(), token);
        }
    }

    #[test]
    fn unknown_tokens_are_rejected() {
        assert!(Shortcut::parse("ctrl+nosuchkey").is_none());
        assert!(Shortcut::parse("").is_none());
    }

    #[test]
    fn defaults_are_complete_and_conflict_free() {
        let settings = Settings::default();
        for action in Action::ALL {
            assert!(settings.shortcut(action).is_some(), "{action:?}");
        }
        assert!(settings.conflicts().is_empty());
    }

    #[test]
    fn unbinding_disables_the_action() {
        let mut settings = Settings::default();
        settings.unbind(Action::PlayPause);
        assert!(settings.shortcut(Action::PlayPause).is_none());
        assert!(settings.conflicts().is_empty());

        // Survives a save/load cycle instead of silently reverting.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        settings.save(&path).unwrap();
        assert!(Settings::load(&path).shortcut(Action::PlayPause).is_none());
    }

    #[test]
    fn rebinding_detects_conflicts() {
        let mut settings = Settings::default();
        let play = settings.shortcut(Action::PlayPause).unwrap();
        settings.set(Action::Stop, play);
        let conflicts = settings.conflicts();
        assert!(conflicts.contains(&Action::PlayPause));
        assert!(conflicts.contains(&Action::Stop));
    }

    #[test]
    fn saved_settings_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut settings = Settings::default();
        settings.set(Action::NextTrack, Shortcut::parse("alt+n").unwrap());
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path);
        assert_eq!(loaded.shortcut(Action::NextTrack).unwrap().token(), "alt+n");
        assert_eq!(
            loaded.shortcut(Action::PlayPause).unwrap().token(),
            "space",
            "unlisted actions keep their defaults"
        );

        // Corrupt file must not take the app down.
        std::fs::write(&path, "{ not json").unwrap();
        let fallback = Settings::load(&path);
        assert_eq!(
            fallback.shortcut(Action::PlayPause).unwrap().token(),
            "space"
        );
    }

    #[test]
    fn equalizer_and_skin_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut settings = Settings::default();
        settings.equalizer.enabled = true;
        settings.equalizer.gains_db[2] = 4.5;
        settings.equalizer.preamp_db = -2.0;
        settings.skin = crate::theme::Skin::Midnight;
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path);
        assert!(loaded.equalizer.enabled);
        assert_eq!(loaded.equalizer.gains_db[2], 4.5);
        assert_eq!(loaded.equalizer.preamp_db, -2.0);
        assert_eq!(loaded.skin, crate::theme::Skin::Midnight);
    }

    #[test]
    fn folded_browser_sections_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut settings = Settings::default();
        settings.folded.genre = true;
        settings.folded.playlists = true;
        settings.queue_open = false;
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path);
        assert!(loaded.folded.genre);
        assert!(!loaded.folded.artist);
        assert!(loaded.folded.playlists);
        assert!(!loaded.queue_open);

        // Files written before the feature existed start fully expanded, with
        // the queue panel visible.
        std::fs::write(&path, r#"{"shortcuts":{"play_pause":"space"}}"#).unwrap();
        let old = Settings::load(&path);
        assert_eq!(old.folded, FoldedSections::default());
        assert!(old.queue_open);
    }

    #[test]
    fn f2_moves_from_the_tag_editor_to_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"shortcuts":{"play_pause":"space","edit_tags":"f2"}}"#,
        )
        .unwrap();

        let loaded = Settings::load(&path);
        assert_eq!(loaded.shortcut(Action::Rename).unwrap().token(), "f2");
        assert_eq!(
            loaded.shortcut(Action::EditTags).unwrap().token(),
            "ctrl+f2"
        );
        assert!(loaded.conflicts().is_empty());
    }

    #[test]
    fn old_settings_without_new_fields_keep_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"shortcuts":{"play_pause":"space"}}"#).unwrap();

        let loaded = Settings::load(&path);
        assert!(!loaded.equalizer.enabled);
        assert_eq!(loaded.skin, crate::theme::Skin::default());
        assert_eq!(loaded.shortcut(Action::PlayPause).unwrap().token(), "space");
    }
}
