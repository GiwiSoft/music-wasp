use crate::settings::{Action, Session, SessionView, Settings, Shortcut, WindowGeometry};
use musicwasp_core::autotag::{self, Candidate, LocalTrack, Release};
use musicwasp_core::dsp::Equalizer;
use musicwasp_core::smart::{Definition, Field, Op, Rule, SmartPlaylist};
use musicwasp_core::Playlist;
use musicwasp_core::{Filter, Library, Player, SortKey, TagEdit, Track};
use notify::Watcher as _;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Playlist(i64),
    Smart(i64),
}

impl From<View> for SessionView {
    fn from(view: View) -> Self {
        match view {
            View::Library => SessionView::Library,
            View::Playlist(id) => SessionView::Playlist(id),
            View::Smart(id) => SessionView::Smart(id),
        }
    }
}

impl From<SessionView> for View {
    fn from(view: SessionView) -> Self {
        match view {
            SessionView::Library => View::Library,
            SessionView::Playlist(id) => View::Playlist(id),
            SessionView::Smart(id) => View::Smart(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Keybindings,
    About,
}

/// Drag payload for track rows: the ids plus where the drag started, so a drop
/// on another playlist can move instead of copy.
#[derive(Debug, Clone)]
pub struct DragTracks {
    pub ids: Vec<i64>,
    pub from_playlist: Option<i64>,
}

/// Drag payload for reordering the play queue.
#[derive(Debug, Clone, Copy)]
pub struct QueueDrag {
    pub index: usize,
}

/// Editing state for a new smart playlist.
pub struct SmartDraft {
    pub name: String,
    pub definition: Definition,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn label(self) -> &'static str {
        match self {
            RepeatMode::Off => "Repeat: off",
            RepeatMode::All => "Repeat: all",
            RepeatMode::One => "Repeat: one",
        }
    }
    pub fn next(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

pub struct EditState {
    /// One file for a normal edit, many for a batch edit.
    pub paths: Vec<PathBuf>,
    pub edit: TagEdit,
    /// Batch mode: empty fields mean "leave unchanged" instead of "clear".
    pub batch: bool,
    pub status: Option<String>,
}

/// A track-list column that F2 can rename in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackField {
    #[default]
    Title,
    Artist,
    Album,
    Genre,
    Year,
    TrackNo,
}

impl TrackField {
    pub fn label(self) -> &'static str {
        match self {
            TrackField::Title => "Title",
            TrackField::Artist => "Artist",
            TrackField::Album => "Album",
            TrackField::Genre => "Genre",
            TrackField::Year => "Year",
            TrackField::TrackNo => "Track number",
        }
    }

    /// Current value as shown in the table.
    pub fn value(self, track: &Track) -> String {
        match self {
            TrackField::Title => track.title.clone(),
            TrackField::Artist => track.artist.clone(),
            TrackField::Album => track.album.clone(),
            TrackField::Genre => track.genre.clone(),
            TrackField::Year => track.year.map(|y| y.to_string()).unwrap_or_default(),
            TrackField::TrackNo => track.track_no.map(|n| n.to_string()).unwrap_or_default(),
        }
    }

    /// Apply the edited text to `edit`. Empty numeric fields clear the value.
    pub fn apply(self, edit: &mut TagEdit, text: &str) -> Result<(), String> {
        let text = text.trim();
        match self {
            TrackField::Title => edit.title = text.to_string(),
            TrackField::Artist => edit.artist = text.to_string(),
            TrackField::Album => edit.album = text.to_string(),
            TrackField::Genre => edit.genre = text.to_string(),
            TrackField::Year => {
                edit.year = parse_number(text, "year", 0..=9999)?.map(|n| n as i32);
            }
            TrackField::TrackNo => {
                edit.track_no = parse_number(text, "track number", 0..=999)?.map(|n| n as i32);
            }
        }
        Ok(())
    }
}

/// `None` for an empty field, otherwise the parsed number within `range`.
fn parse_number(
    text: &str,
    what: &str,
    range: std::ops::RangeInclusive<i64>,
) -> Result<Option<i64>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let value: i64 = text
        .parse()
        .map_err(|_| format!("{what} must be a whole number"))?;
    if !range.contains(&value) {
        return Err(format!(
            "{what} must be between {} and {}",
            range.start(),
            range.end()
        ));
    }
    Ok(Some(value))
}

/// What the rename dialog is currently editing.
#[derive(Debug, Clone)]
pub enum RenameTarget {
    Track { id: i64, field: TrackField },
    Playlist(i64),
    Smart(i64),
}

impl RenameTarget {
    pub fn title(&self) -> String {
        match self {
            RenameTarget::Track { field, .. } => format!("Rename {}", field.label().to_lowercase()),
            RenameTarget::Playlist(_) => "Rename playlist".to_string(),
            RenameTarget::Smart(_) => "Rename smart playlist".to_string(),
        }
    }
}

pub struct RenameState {
    pub target: RenameTarget,
    pub text: String,
    pub error: Option<String>,
    /// Focus the text box on the next frame.
    pub request_focus: bool,
}

/// Pane that received the last click; F2 renames in that context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Tracks,
    Browser,
}

/// A region that accepts audio files dropped from the OS file manager.
/// `playlist` is `None` for the whole library ("All music").
#[derive(Debug, Clone, Copy)]
pub struct OsDropZone {
    pub playlist: Option<i64>,
    pub rect: egui::Rect,
}

// ---------------------------------------------------------------- auto-tag

#[derive(Debug)]
pub enum AutoTagMsg {
    SearchDone(Result<Vec<Candidate>, String>),
    ReleaseLoaded {
        release_id: String,
        result: Result<Release, String>,
    },
    Applied {
        written: usize,
        errors: Vec<String>,
        paths: Vec<PathBuf>,
        cover: bool,
        cover_requested: bool,
    },
}

pub enum AutoTagStage {
    Editing,
    Searching,
    Results(Vec<Candidate>),
    Applying,
    Done(String),
}

pub struct AutoTagState {
    pub artist: String,
    pub album: String,
    pub stage: AutoTagStage,
    pub selected: usize,
    pub embed_art: bool,
    /// Files included when the dialog was opened. Keeping the snapshot means
    /// changing the library selection cannot silently change the preview.
    pub targets: Vec<Track>,
    /// File-to-release-track preview, rebuilt when a release finishes loading.
    pub mappings: Vec<AutoTagMapping>,
    /// Release details for the selected candidate.
    pub release: Option<Release>,
    pub release_id: Option<String>,
    pub loading_release: bool,
    pub release_error: Option<String>,
    pub rx: Receiver<AutoTagMsg>,
    pub tx: Sender<AutoTagMsg>,
}

/// One preview row: local file and the release track it will become.
pub struct AutoTagMapping {
    pub track: Track,
    pub release_index: Option<usize>,
}

impl AutoTagMapping {
    /// Diff against the release track, for the preview column.
    pub fn changes<'a>(&'a self, release: &'a Release) -> Vec<(&'static str, String, String)> {
        let Some(track) = self
            .release_index
            .and_then(|index| release.tracks.get(index))
        else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        let mut compare = |field: &'static str, old: String, new: String| {
            if old != new {
                changes.push((field, old, new));
            }
        };
        compare("Title", self.track.title.clone(), track.title.clone());
        compare(
            "Artist",
            self.track.artist.clone(),
            if track.artist.is_empty() {
                self.track.artist.clone()
            } else {
                track.artist.clone()
            },
        );
        compare("Album", self.track.album.clone(), release.title.clone());
        compare(
            "Album artist",
            self.track.album_artist.clone(),
            release.album_artist.clone(),
        );
        compare(
            "Year",
            self.track.year.map(|y| y.to_string()).unwrap_or_default(),
            release
                .year
                .map(|y| y.to_string())
                .or_else(|| self.track.year.map(|y| y.to_string()))
                .unwrap_or_default(),
        );
        compare(
            "Genre",
            self.track.genre.clone(),
            release
                .genre
                .clone()
                .unwrap_or_else(|| self.track.genre.clone()),
        );
        compare(
            "Track",
            self.track
                .track_no
                .map(|n| n.to_string())
                .unwrap_or_default(),
            track.number.to_string(),
        );
        compare(
            "Disc",
            self.track
                .disc_no
                .map(|n| n.to_string())
                .unwrap_or_default(),
            track.disc.to_string(),
        );
        changes
    }
}

/// Results of a library hygiene scan: rows whose files vanished, and groups of
/// tracks that share the same artist/title/album.
pub struct HygieneState {
    pub missing: Vec<Track>,
    pub duplicate_groups: Vec<Vec<Track>>,
    pub selected: BTreeSet<i64>,
    /// Ids awaiting confirmation before they are removed from the database.
    pub pending: Option<Vec<i64>>,
    pub status: Option<String>,
}

impl AutoTagState {
    pub fn new(artist: String, album: String, targets: Vec<Track>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            artist,
            album,
            stage: AutoTagStage::Editing,
            selected: 0,
            embed_art: true,
            targets,
            mappings: Vec::new(),
            release: None,
            release_id: None,
            loading_release: false,
            release_error: None,
            rx,
            tx,
        }
    }
}

/// Directory chooser state (in-app, so no desktop-portal dependency).
pub struct FolderPicker {
    pub current: PathBuf,
    pub entries: Vec<PathBuf>,
    pub show_hidden: bool,
    pub error: Option<String>,
}

impl FolderPicker {
    pub fn new(start: PathBuf) -> Self {
        let mut picker = Self {
            current: start,
            entries: Vec::new(),
            show_hidden: false,
            error: None,
        };
        if !picker.current.is_dir() {
            picker.current = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        }
        picker.reload();
        picker
    }

    pub fn reload(&mut self) {
        match list_subdirs(&self.current, self.show_hidden) {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }
    }

    pub fn navigate(&mut self, to: PathBuf) {
        if to.is_dir() {
            self.current = to;
            self.reload();
        }
    }
}

/// Selection rules shared by the table and the tests.
///
/// Plain click selects one row, Ctrl/Cmd toggles a row, Shift selects the range
/// from the anchor to the clicked row.
pub fn apply_click(
    selected: &mut BTreeSet<i64>,
    anchor: &mut Option<i64>,
    view: &[i64],
    index: usize,
    ctrl: bool,
    shift: bool,
) {
    let Some(&id) = view.get(index) else {
        return;
    };

    if ctrl {
        if !selected.insert(id) {
            selected.remove(&id);
        }
        *anchor = Some(id);
        return;
    }

    if shift {
        if let Some(anchor_id) = *anchor {
            if let Some(anchor_idx) = view.iter().position(|&v| v == anchor_id) {
                let (from, to) = if anchor_idx <= index {
                    (anchor_idx, index)
                } else {
                    (index, anchor_idx)
                };
                *selected = view[from..=to].iter().copied().collect();
                return;
            }
        }
    }

    selected.clear();
    selected.insert(id);
    *anchor = Some(id);
}

/// Fields shared by every track; differing fields come back empty so a batch
/// edit leaves them alone.
/// Best release track for one local file: exact disc/track, then title, then
/// the closest duration, finally the first track.
/// Build the default mapping preview for the current targets.
fn suggested_mappings(targets: &[Track], release: &Release) -> Vec<AutoTagMapping> {
    let local: Vec<LocalTrack> = targets
        .iter()
        .map(|track| LocalTrack {
            path: PathBuf::from(&track.path),
            disc_no: track.disc_no,
            track_no: track.track_no,
            title: track.title.clone(),
            duration_secs: track.duration_secs,
        })
        .collect();
    let suggested = autotag::suggest(&local, release);
    targets
        .iter()
        .cloned()
        .zip(suggested)
        .map(|(track, release_index)| AutoTagMapping {
            track,
            release_index,
        })
        .collect()
}

/// Turn the preview rows into the matches that will actually be written.
fn mapping_matches(mappings: &[AutoTagMapping], release: &Release) -> Vec<autotag::Match> {
    mappings
        .iter()
        .filter_map(|mapping| {
            let track = mapping
                .release_index
                .and_then(|index| release.tracks.get(index))?;
            Some(autotag::Match {
                path: PathBuf::from(&mapping.track.path),
                track: track.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
fn best_release_track(local: &Track, release: &Release) -> usize {
    let disc = local.disc_no.unwrap_or(1);
    if let Some(number) = local.track_no {
        if let Some(index) = release
            .tracks
            .iter()
            .position(|t| t.number as i32 == number && t.disc as i32 == disc)
        {
            return index;
        }
        if let Some(index) = release
            .tracks
            .iter()
            .position(|t| t.number as i32 == number)
        {
            return index;
        }
    }

    let title = local.title.trim().to_lowercase();
    if !title.is_empty() {
        if let Some(index) = release
            .tracks
            .iter()
            .position(|t| t.title.trim().to_lowercase() == title)
        {
            return index;
        }
        if let Some(index) = release.tracks.iter().position(|t| {
            let candidate = t.title.trim().to_lowercase();
            !candidate.is_empty() && (candidate.contains(&title) || title.contains(&candidate))
        }) {
            return index;
        }
    }

    let duration_ms = local.duration_secs as i64 * 1000;
    release
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(index, t)| {
            t.length_ms
                .map(|ms| (index, (ms as i64 - duration_ms).abs()))
        })
        .min_by_key(|(_, delta)| *delta)
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// Insert `ids` right after `pos` (or at the end when idle).
fn queue_insert_next(queue: &mut Vec<i64>, pos: Option<usize>, ids: &[i64]) {
    let at = pos
        .map(|pos| (pos + 1).min(queue.len()))
        .unwrap_or(queue.len());
    for (offset, id) in ids.iter().enumerate() {
        queue.insert((at + offset).min(queue.len()), *id);
    }
}

/// Drop queue entries that no longer exist, keeping the current position on the
/// same track when it survived.
fn restore_queue(
    queue: &[i64],
    pos: Option<usize>,
    valid: &BTreeSet<i64>,
) -> (Vec<i64>, Option<usize>) {
    let mut restored = Vec::with_capacity(queue.len());
    let mut restored_pos = None;
    for (index, id) in queue.iter().copied().enumerate() {
        if !valid.contains(&id) {
            continue;
        }
        if pos == Some(index) {
            restored_pos = Some(restored.len());
        }
        restored.push(id);
    }
    (restored, restored_pos)
}

/// Drop queue entries for tracks removed from the library and keep the current
/// entry selected. Returns the new index and whether the current entry went
/// away.
fn remove_tracks_from_queue(
    queue: &mut Vec<i64>,
    pos: Option<usize>,
    removed: &BTreeSet<i64>,
) -> (Option<usize>, bool) {
    let current = pos.and_then(|pos| queue.get(pos).copied());
    let current_removed = current.is_some_and(|id| removed.contains(&id));
    queue.retain(|id| !removed.contains(id));
    let new_pos = if current_removed {
        None
    } else {
        current.and_then(|id| queue.iter().position(|queued| *queued == id))
    };
    (new_pos, current_removed)
}

/// Reorder an upcoming queue entry, dropping it before `to`. The playing entry
/// and anything before it are left alone. Returns true when the order changed.
fn queue_reorder(queue: &mut Vec<i64>, pos: Option<usize>, from: usize, to: usize) -> bool {
    let len = queue.len();
    if from >= len || to > len || from == to {
        return false;
    }
    if pos.is_some_and(|pos| from <= pos || to <= pos) {
        return false;
    }
    let id = queue.remove(from);
    let dest = if from < to { to - 1 } else { to };
    queue.insert(dest.min(queue.len()), id);
    true
}

pub fn common_fields(tracks: &[Track]) -> TagEdit {
    let same_text = |field: fn(&Track) -> &str| -> String {
        let Some(first) = tracks.first() else {
            return String::new();
        };
        let value = field(first);
        if tracks.iter().all(|t| field(t) == value) {
            value.to_string()
        } else {
            String::new()
        }
    };
    let same_number = |field: fn(&Track) -> Option<i32>| -> Option<i32> {
        let first = tracks.first()?;
        let value = field(first);
        if tracks.iter().all(|t| field(t) == value) {
            value
        } else {
            None
        }
    };

    TagEdit {
        title: same_text(|t| &t.title),
        artist: same_text(|t| &t.artist),
        album: same_text(|t| &t.album),
        album_artist: same_text(|t| &t.album_artist),
        genre: same_text(|t| &t.genre),
        year: same_number(|t| t.year),
        track_no: same_number(|t| t.track_no),
        disc_no: same_number(|t| t.disc_no),
    }
}

/// Sub-directories of `dir`, sorted, hidden ones optionally included.
pub fn list_subdirs(dir: &std::path::Path, show_hidden: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|path| {
            show_hidden
                || !path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(dirs)
}

pub struct App {
    pub lib: Library,
    pub player: Option<Player>,
    pub audio_error: Option<String>,

    // Column browser state
    pub genres: Vec<String>,
    pub artists: Vec<String>,
    pub albums: Vec<String>,
    pub sel_genre: Option<String>,
    pub sel_artist: Option<String>,
    pub sel_album: Option<String>,

    // Track list
    pub tracks: Vec<Track>,
    pub sort: SortKey,
    pub sort_asc: bool,
    pub search: String,
    /// Selected track ids; several at once for batch operations.
    pub selected: BTreeSet<i64>,
    /// Track the next shift-click extends from.
    pub anchor: Option<i64>,

    // Queue
    pub queue: Vec<i64>,
    pub queue_pos: Option<usize>,
    pub repeat: RepeatMode,
    pub shuffle: bool,

    // Transport UI
    pub seek_drag: Option<f64>,
    pub volume: f32,
    pub volume_sent: f32,

    // Misc
    pub scan_path: String,
    pub message: String,
    pub art: Option<(String, egui::TextureHandle)>,
    pub art_key: Option<String>,
    /// Toolbar brand mark.
    pub logo: Option<egui::TextureHandle>,
    /// Full-window now-playing view instead of browser + track list.
    pub plain_view: bool,
    pub playing_id: Option<i64>,
    pub editing: Option<EditState>,
    /// Inline rename dialog; F2 opens it for the focused pane.
    pub rename: Option<RenameState>,
    /// Field clicked last in the track table, renamed by F2.
    pub rename_field: TrackField,
    pub focus: Focus,
    /// Drop targets from the last browser frame, for OS file drops.
    pub os_drop_zones: Vec<OsDropZone>,
    /// Playlist most recently under an OS file drag, used if the drop event
    /// arrives without a final pointer position.
    pub os_drop_hover: Option<Option<i64>>,
    pub view: View,
    pub playlists: Vec<Playlist>,
    pub smart_playlists: Vec<SmartPlaylist>,
    pub smart_dialog: Option<SmartDraft>,
    pub playlist_dialog: Option<String>,
    pub picker: Option<FolderPicker>,
    pub autotag: Option<AutoTagState>,
    pub show_hygiene: bool,
    pub hygiene: Option<HygieneState>,
    /// OS watcher for scanned folders; `None` when disabled or idle.
    watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Option<Receiver<PathBuf>>,
    watch_pending: BTreeSet<PathBuf>,
    watch_deadline: Option<std::time::Instant>,
    watch_dirty: bool,
    watched_folders: Vec<PathBuf>,
    /// Linux desktop integration (MPRIS2) and its last publish time.
    #[cfg(target_os = "linux")]
    mpris: Option<crate::mpris::MprisService>,
    #[cfg(target_os = "linux")]
    mpris_last: Option<std::time::Instant>,
    dirty_browser: bool,
    dirty_tracks: bool,

    // Settings
    pub settings: Settings,
    pub settings_path: PathBuf,
    /// Last session written to disk, used to avoid rewriting settings each frame.
    last_session: Session,
    last_session_save: std::time::Instant,
    /// Last known window geometry; restored at startup and written on exit.
    window_geometry: Option<WindowGeometry>,
    /// Screenshot runs must be deterministic, so they skip session restore/save.
    session_enabled: bool,
    pub show_settings: bool,
    pub settings_tab: SettingsTab,
    pub show_equalizer: bool,
    /// Equalizer state last pushed to the engine, so changes are cheap.
    pub eq_applied: Equalizer,
    /// Action currently waiting for the user to press a key.
    pub capture: Option<Action>,
    pub focus_search: bool,
    /// Guards against the click that started a rebind also re-triggering it
    /// when the pressed key activates the focused button.
    pub captured_this_frame: bool,

    // Debug: MUSICWASP_SCREENSHOT=/path/out.png renders N frames, saves, exits.
    frames: u32,
    shot_path: Option<String>,
    shot_requested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings_path = Settings::path();
        let mut settings = Settings::load(&settings_path);
        settings.equalizer = settings.equalizer.clamped();
        crate::icons::install(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx, settings.skin);

        let lib = Library::open().expect("cannot open library database");
        let (player, audio_error) = match Player::new() {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e.to_string())),
        };

        let scan_path = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/music".to_string());

        let eq_applied = settings.equalizer;

        let shot_path = std::env::var("MUSICWASP_SCREENSHOT").ok();
        let session_enabled = shot_path.is_none();
        let session = if session_enabled {
            settings.session.clone()
        } else {
            Session::default()
        };
        let volume = session.volume.unwrap_or(0.8).clamp(0.0, 1.5);
        let window_geometry = session.window;
        let anchor = session.selected.first().copied();
        #[cfg(target_os = "linux")]
        let mpris = if session_enabled {
            crate::mpris::MprisService::start(cc.egui_ctx.clone())
        } else {
            None
        };

        let mut app = Self {
            lib,
            player,
            audio_error,
            genres: Vec::new(),
            artists: Vec::new(),
            albums: Vec::new(),
            sel_genre: None,
            sel_artist: None,
            sel_album: None,
            tracks: Vec::new(),
            sort: SortKey::Artist,
            sort_asc: true,
            search: session.search.clone(),
            selected: session.selected.iter().copied().collect(),
            anchor,
            queue: session.queue.clone(),
            queue_pos: session.queue_pos,
            repeat: RepeatMode::Off,
            shuffle: false,
            seek_drag: None,
            volume,
            volume_sent: -1.0,
            scan_path,
            message: String::new(),
            art: None,
            art_key: None,
            logo: None,
            plain_view: std::env::var("MUSICWASP_PLAIN").is_ok(),
            playing_id: None,
            editing: None,
            rename: None,
            rename_field: TrackField::default(),
            focus: Focus::default(),
            os_drop_zones: Vec::new(),
            os_drop_hover: None,
            view: session.view.into(),
            playlists: Vec::new(),
            smart_playlists: Vec::new(),
            smart_dialog: None,
            playlist_dialog: None,
            picker: None,
            autotag: None,
            show_hygiene: false,
            hygiene: None,
            watcher: None,
            watch_rx: None,
            watch_pending: BTreeSet::new(),
            watch_deadline: None,
            watch_dirty: false,
            watched_folders: Vec::new(),
            #[cfg(target_os = "linux")]
            mpris,
            #[cfg(target_os = "linux")]
            mpris_last: None,
            dirty_browser: true,
            dirty_tracks: true,
            settings,
            settings_path,
            last_session: Session::default(),
            last_session_save: std::time::Instant::now(),
            window_geometry,
            session_enabled,
            show_settings: false,
            settings_tab: SettingsTab::default(),
            show_equalizer: false,
            eq_applied,
            capture: None,
            focus_search: false,
            captured_this_frame: false,
            frames: 0,
            shot_path,
            shot_requested: false,
        };
        app.refresh_browser();
        app.refresh_tracks();
        if app.session_enabled {
            app.restore_session(&session);
        }
        app.last_session = app.capture_session();
        app.refresh_brand_icon(&cc.egui_ctx);
        if let Some(player) = &app.player {
            player.set_equalizer(app.settings.equalizer);
        }
        if app.session_enabled {
            app.restart_watcher(&cc.egui_ctx);
        }
        app
    }

    /// Selected ids in the order they appear on screen.
    pub fn selected_in_view(&self) -> Vec<i64> {
        self.tracks
            .iter()
            .filter(|t| self.selected.contains(&t.id))
            .map(|t| t.id)
            .collect()
    }

    /// Click behaviour for a row: plain selects, Ctrl toggles, Shift extends.
    pub fn click_row(&mut self, index: usize, ctrl: bool, shift: bool) {
        let view: Vec<i64> = self.tracks.iter().map(|t| t.id).collect();
        apply_click(
            &mut self.selected,
            &mut self.anchor,
            &view,
            index,
            ctrl,
            shift,
        );
    }

    /// Make sure the row is part of the selection before a context-menu action.
    pub fn select_row_for_action(&mut self, index: usize) {
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        if !self.selected.contains(&track.id) {
            self.click_row(index, false, false);
        }
    }

    /// Select exactly this row (single-file actions like rename and auto-tag).
    pub fn select_only_row(&mut self, index: usize) {
        if self.tracks.get(index).is_some() {
            self.click_row(index, false, false);
        }
    }

    pub fn select_all_in_view(&mut self) {
        self.selected = self.tracks.iter().map(|t| t.id).collect();
        if self.anchor.is_none() {
            self.anchor = self.tracks.first().map(|t| t.id);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn filter(&self) -> Filter {
        Filter {
            genre: self.sel_genre.clone(),
            artist: self.sel_artist.clone(),
            album: self.sel_album.clone(),
            text: self.search.clone(),
        }
    }

    pub fn refresh_browser(&mut self) {
        self.genres = self.lib.genres().unwrap_or_default();

        // Keep selections valid against the current data set.
        if let Some(g) = &self.sel_genre {
            if !self.genres.contains(g) {
                self.sel_genre = None;
            }
        }
        self.artists = self
            .lib
            .artists(self.sel_genre.as_deref())
            .unwrap_or_default();
        if let Some(a) = &self.sel_artist {
            if !self.artists.contains(a) {
                self.sel_artist = None;
            }
        }
        self.albums = self
            .lib
            .albums(self.sel_artist.as_deref(), self.sel_genre.as_deref())
            .unwrap_or_default();
        if let Some(al) = &self.sel_album {
            if !self.albums.contains(al) {
                self.sel_album = None;
            }
        }
        self.playlists = self.lib.playlists().unwrap_or_default();
        self.smart_playlists = self.lib.smart_playlists().unwrap_or_default();
        match self.view {
            View::Playlist(id) if !self.playlists.iter().any(|p| p.id == id) => {
                self.view = View::Library;
            }
            View::Smart(id) if !self.smart_playlists.iter().any(|p| p.id == id) => {
                self.view = View::Library;
            }
            _ => {}
        }
        self.dirty_browser = false;
        self.dirty_tracks = true;
    }

    pub fn refresh_tracks(&mut self) {
        let result = match self.view {
            View::Library => self.lib.query(&self.filter(), self.sort, self.sort_asc),
            View::Playlist(id) => self.lib.playlist_tracks(id),
            View::Smart(id) => self.lib.smart_playlist_tracks(id),
        };
        match result {
            Ok(t) => self.tracks = t,
            Err(e) => self.message = format!("query failed: {e}"),
        }
        self.dirty_tracks = false;
    }

    /// Revalidate ids loaded from `settings.json` and put the last queue entry
    /// back at its saved position. Playback always starts stopped: restoring a
    /// track used to make the play/pause icon flip while the audio thread
    /// switched from playing to paused.
    fn restore_session(&mut self, session: &Session) {
        let valid_queue: BTreeSet<i64> = self
            .lib
            .tracks_by_ids(&session.queue)
            .unwrap_or_default()
            .into_iter()
            .map(|track| track.id)
            .collect();
        let (queue, queue_pos) = restore_queue(&session.queue, session.queue_pos, &valid_queue);
        self.queue = queue;
        self.queue_pos = queue_pos;

        let valid_selected: BTreeSet<i64> = self
            .lib
            .tracks_by_ids(&session.selected)
            .unwrap_or_default()
            .into_iter()
            .map(|track| track.id)
            .collect();
        self.selected.retain(|id| valid_selected.contains(id));
        if self.anchor.is_none() {
            self.anchor = self.selected.iter().next().copied();
        }

        // Always start with a stopped transport. The queue position and volume
        // still come back, but no track is loaded until the user presses play.
        self.playing_id = None;
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Snapshot what should come back on the next launch.
    fn capture_session(&self) -> Session {
        Session {
            view: self.view.into(),
            search: self.search.clone(),
            selected: self.selected.iter().copied().collect(),
            volume: Some(self.volume),
            queue: self.queue.clone(),
            queue_pos: self.queue_pos,
            window: self.window_geometry,
        }
    }

    /// Store the window rect. Wayland reports no screen position, so only the
    /// size is remembered there.
    fn capture_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, viewport_rect) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect,
                viewport.outer_rect,
                input.viewport_rect(),
            )
        });
        let size = inner
            .map(|rect| rect.size())
            .unwrap_or_else(|| viewport_rect.size());
        if size.x < 200.0 || size.y < 200.0 {
            return; // minimized or not known yet
        }
        let (x, y) = outer
            .map(|rect| (Some(rect.min.x), Some(rect.min.y)))
            .unwrap_or((None, None));
        self.window_geometry = Some(WindowGeometry {
            width: size.x,
            height: size.y,
            x,
            y,
        });
    }

    /// Write the session when it changed, at most every few seconds so dragging
    /// the volume or typing in search does not hammer the disk.
    fn persist_session(&mut self, force: bool) {
        if !self.session_enabled {
            return;
        }
        let session = self.capture_session();
        if !force {
            if session == self.last_session {
                return;
            }
            if self.last_session_save.elapsed() < std::time::Duration::from_secs(5) {
                return;
            }
        }
        self.settings.session = session.clone();
        self.persist_settings();
        self.last_session = session;
        self.last_session_save = std::time::Instant::now();
    }

    pub fn view_title(&self) -> String {
        match self.view {
            View::Library => "Library".to_string(),
            View::Playlist(id) => self
                .playlists
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("Playlist · {}", p.name))
                .unwrap_or_else(|| "Library".to_string()),
            View::Smart(id) => self
                .smart_playlists
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("Smart · {}", p.name))
                .unwrap_or_else(|| "Library".to_string()),
        }
    }

    /// Save the currently displayed tracks as a new playlist.
    pub fn commit_playlist_dialog(&mut self) {
        let Some(name) = self.playlist_dialog.clone() else {
            return;
        };
        let selected = self.selected_in_view();
        let ids: Vec<i64> = if selected.is_empty() {
            self.tracks.iter().map(|t| t.id).collect()
        } else {
            selected
        };
        match self.lib.create_playlist(&name, &ids) {
            Ok(id) => {
                self.message = format!("playlist \"{}\" saved ({} tracks)", name.trim(), ids.len());
                self.playlist_dialog = None;
                self.view = View::Playlist(id);
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("playlist failed: {e}"),
        }
    }

    pub fn delete_current_playlist(&mut self) {
        let result = match self.view {
            View::Playlist(id) => self.lib.delete_playlist(id),
            View::Smart(id) => self.lib.delete_smart_playlist(id),
            View::Library => return,
        };
        match result {
            Ok(()) => {
                self.view = View::Library;
                self.message = "playlist deleted".to_string();
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("delete failed: {e}"),
        }
    }

    /// Playlist id when the current view is an editable (manual) playlist.
    pub fn editable_playlist(&self) -> Option<i64> {
        match self.view {
            View::Playlist(id) => Some(id),
            _ => None,
        }
    }

    pub fn remove_selection_from_playlist(&mut self) {
        let Some(playlist_id) = self.editable_playlist() else {
            return;
        };
        let ids = self.selected_in_view();
        if ids.is_empty() {
            return;
        }
        match self.lib.remove_from_playlist(playlist_id, &ids) {
            Ok(n) => {
                self.message = format!("removed {n} track(s) from the playlist");
                self.selected.clear();
                self.mark_tracks_dirty();
            }
            Err(e) => self.message = format!("remove failed: {e}"),
        }
    }

    /// Move the selection one row up (`delta = -1`) or down.
    pub fn move_selection(&mut self, delta: i32) {
        let Some(playlist_id) = self.editable_playlist() else {
            return;
        };
        let ids = self.selected_in_view();
        if ids.is_empty() {
            return;
        }
        let indices: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| self.selected.contains(&t.id))
            .map(|(i, _)| i)
            .collect();
        let (first, last) = (indices[0], *indices.last().unwrap());

        let before = if delta < 0 {
            if first == 0 {
                return;
            }
            Some(self.tracks[first - 1].id)
        } else {
            self.tracks.get(last + 2).map(|t| t.id)
        };

        match self.lib.move_in_playlist(playlist_id, &ids, before) {
            Ok(()) => self.mark_tracks_dirty(),
            Err(e) => self.message = format!("move failed: {e}"),
        }
    }

    /// Drop dragged rows onto a row of the current playlist (reorder).
    pub fn drop_in_playlist(&mut self, index: usize, drag: &DragTracks) {
        let Some(playlist_id) = self.editable_playlist() else {
            return;
        };
        if drag.from_playlist != Some(playlist_id) {
            return;
        }
        let Some(before) = self.tracks.get(index).map(|t| t.id) else {
            return;
        };
        if drag.ids.contains(&before) {
            return;
        }
        match self
            .lib
            .move_in_playlist(playlist_id, &drag.ids, Some(before))
        {
            Ok(()) => {
                self.selected = drag.ids.iter().copied().collect();
                self.mark_tracks_dirty();
            }
            Err(e) => self.message = format!("reorder failed: {e}"),
        }
    }

    // ---- smart playlists ----

    pub fn open_smart_dialog(&mut self) {
        let mut definition = Definition::default();
        definition.rules.push(Rule::new(Field::Genre, Op::Contains));
        self.smart_dialog = Some(SmartDraft {
            name: format!("Smart playlist {}", self.smart_playlists.len() + 1),
            definition,
            status: None,
        });
    }

    pub fn commit_smart_dialog(&mut self) {
        let Some(draft) = &mut self.smart_dialog else {
            return;
        };
        let name = draft.name.trim().to_string();
        if name.is_empty() {
            draft.status = Some("give the playlist a name".to_string());
            return;
        }
        match self.lib.create_smart_playlist(&name, &draft.definition) {
            Ok(id) => {
                self.message = format!("smart playlist \"{name}\" created");
                self.smart_dialog = None;
                self.view = View::Smart(id);
                self.mark_browser_dirty();
            }
            Err(e) => draft.status = Some(format!("create failed: {e}")),
        }
    }

    /// Add the current selection (or an explicit dragged list) to a playlist.
    pub fn add_to_playlist(&mut self, playlist_id: i64, ids: &[i64]) {
        self.transfer_to_playlist(playlist_id, ids, None);
    }

    /// Add or move tracks into a playlist. When `from` is another playlist the
    /// rows are moved, which is what drag-and-drop between playlists means.
    pub fn transfer_to_playlist(&mut self, playlist_id: i64, ids: &[i64], from: Option<i64>) {
        let ids: Vec<i64> = if ids.is_empty() {
            self.selected_in_view()
        } else {
            ids.to_vec()
        };
        if ids.is_empty() {
            self.message = "select a track first".to_string();
            return;
        }
        if from == Some(playlist_id) {
            return;
        }

        // Add first, remove second: a failed add must not lose the tracks.
        match self.lib.add_many_to_playlist(playlist_id, &ids) {
            Ok(_) => {
                if let Some(source) = from {
                    if let Err(e) = self.lib.remove_from_playlist(source, &ids) {
                        self.message = format!("moved, but could not remove from source: {e}");
                        self.mark_browser_dirty();
                        return;
                    }
                    self.message = format!("moved {} track(s)", ids.len());
                } else {
                    self.message = "added to playlist".to_string();
                }
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("add failed: {e}"),
        }
    }

    pub fn mark_browser_dirty(&mut self) {
        self.dirty_browser = true;
    }

    pub fn mark_tracks_dirty(&mut self) {
        self.dirty_tracks = true;
    }

    // ---- library hygiene ----

    /// Open the maintenance window and rescan, so entries deleted outside the
    /// app show up immediately.
    pub fn open_hygiene(&mut self) {
        self.show_hygiene = true;
        self.scan_hygiene();
    }

    /// Collect missing files and duplicate groups. Selections that no longer
    /// appear in either list are dropped.
    pub fn scan_hygiene(&mut self) {
        let missing = match self.lib.missing_tracks() {
            Ok(tracks) => tracks,
            Err(e) => {
                self.message = format!("could not check for missing files: {e}");
                Vec::new()
            }
        };
        let duplicate_groups = match self.lib.duplicate_groups() {
            Ok(groups) => groups,
            Err(e) => {
                self.message = format!("could not check for duplicates: {e}");
                Vec::new()
            }
        };

        let present: BTreeSet<i64> = missing
            .iter()
            .chain(duplicate_groups.iter().flatten())
            .map(|track| track.id)
            .collect();
        let mut selected = self
            .hygiene
            .as_ref()
            .map(|state| state.selected.clone())
            .unwrap_or_default();
        selected.retain(|id| present.contains(id));

        self.hygiene = Some(HygieneState {
            missing,
            duplicate_groups,
            selected,
            pending: None,
            status: None,
        });
    }

    /// Remove rows from the library only. Audio files are never deleted.
    pub fn remove_from_library(&mut self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        let removed: BTreeSet<i64> = ids.iter().copied().collect();
        match self.lib.remove_tracks(ids) {
            Ok(count) => {
                let (queue_pos, current_removed) =
                    remove_tracks_from_queue(&mut self.queue, self.queue_pos, &removed);
                self.queue_pos = queue_pos;
                if current_removed {
                    if let Some(player) = &self.player {
                        player.stop();
                    }
                    self.playing_id = None;
                    self.art = None;
                    self.art_key = None;
                }
                self.selected.retain(|id| !removed.contains(id));
                if self.anchor.is_some_and(|id| removed.contains(&id)) {
                    self.anchor = None;
                }

                self.message =
                    format!("removed {count} track(s) from the library; audio files were kept");
                self.scan_hygiene();
                if let Some(state) = &mut self.hygiene {
                    state.status = Some(self.message.clone());
                }
                self.mark_browser_dirty();
            }
            Err(e) => {
                self.message = format!("could not remove tracks from the library: {e}");
                if let Some(state) = &mut self.hygiene {
                    state.status = Some(self.message.clone());
                }
            }
        }
    }

    // ---- folder watching ----

    /// Enable or disable automatic imports from the watched folders.
    pub fn set_watch_enabled(&mut self, enabled: bool) {
        if self.settings.watch_enabled == enabled {
            return;
        }
        self.settings.watch_enabled = enabled;
        self.watch_dirty = true;
        self.persist_settings();
    }

    /// Stop watching every remembered folder.
    pub fn forget_watch_folders(&mut self) {
        if self.settings.watch_folders.is_empty() {
            return;
        }
        self.settings.watch_folders.clear();
        self.watch_dirty = true;
        self.persist_settings();
    }

    /// Remember a folder the user scanned, so it is watched after a restart.
    fn remember_watch_folder(&mut self, path: &Path) {
        let folder = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !folder.is_dir() {
            return;
        }
        let entry = folder.to_string_lossy().to_string();
        if self.settings.watch_folders.contains(&entry) {
            return;
        }
        self.settings.watch_folders.push(entry);
        self.persist_settings();
        self.watch_dirty = true;
    }

    /// (Re)build the OS watcher. Dropping the old watcher stops its events.
    fn restart_watcher(&mut self, ctx: &egui::Context) {
        self.watcher = None;
        self.watch_rx = None;
        self.watch_pending.clear();
        self.watch_deadline = None;
        self.watched_folders.clear();

        if !self.settings.watch_enabled || !self.session_enabled {
            return;
        }
        let folders: Vec<PathBuf> = self
            .settings
            .watch_folders
            .iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
        if folders.is_empty() {
            return;
        }

        let (tx, rx) = mpsc::channel::<PathBuf>();
        let repaint = ctx.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if !matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Any
            ) {
                return;
            }
            for path in event.paths {
                if tx.send(path).is_err() {
                    break;
                }
            }
            repaint.request_repaint();
        });

        match watcher {
            Ok(mut watcher) => {
                let mut watched = Vec::new();
                for folder in folders {
                    match watcher.watch(&folder, notify::RecursiveMode::Recursive) {
                        Ok(()) => watched.push(folder),
                        Err(e) => {
                            self.message =
                                format!("watcher: cannot watch {}: {e}", folder.display());
                        }
                    }
                }
                self.watcher = Some(watcher);
                self.watch_rx = Some(rx);
                self.watched_folders = watched;
            }
            Err(e) => self.message = format!("watcher could not start: {e}"),
        }
    }

    /// Drain watcher events and run a debounced batch once the filesystem has
    /// settled. Metadata writes can emit several events for one file.
    fn poll_watcher(&mut self, ctx: &egui::Context) {
        if self.watch_rx.is_none() {
            return;
        }
        let mut received = Vec::new();
        let mut disconnected = false;
        if let Some(rx) = &self.watch_rx {
            loop {
                match rx.try_recv() {
                    Ok(path) => received.push(path),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.watch_rx = None;
        }
        if !received.is_empty() {
            for path in received {
                self.watch_pending.insert(path);
            }
            self.watch_deadline =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(700));
        }
        if let Some(deadline) = self.watch_deadline {
            let now = std::time::Instant::now();
            if now >= deadline {
                self.watch_deadline = None;
                self.apply_watched_paths();
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }
    }

    /// Import new files and refresh tags for modified ones. `import_paths`
    /// upserts on the unique path, so repeated events never duplicate rows.
    fn apply_watched_paths(&mut self) {
        let pending = std::mem::take(&mut self.watch_pending);
        if pending.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = pending
            .into_iter()
            .filter(|path| {
                path.is_dir() || (path.is_file() && musicwasp_core::library::is_audio_path(path))
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        match self.lib.import_paths(&paths) {
            Ok(indexed) if !indexed.is_empty() => {
                self.mark_browser_dirty();
                self.message = format!("watcher refreshed {} track(s)", indexed.len());
            }
            Ok(_) => {}
            Err(e) => self.message = format!("watcher import failed: {e}"),
        }
    }

    // ---- Linux desktop integration (MPRIS2) ----

    /// Feed desktop controls with the current track, status and position.
    #[cfg(target_os = "linux")]
    fn publish_mpris(&mut self, ctx: &egui::Context) {
        use crate::mpris::{IDLE_REFRESH, PLAYING_REFRESH};
        use musicwasp_core::PlayerStatus;

        let Some(service) = &self.mpris else {
            return;
        };
        let playing = self
            .player
            .as_ref()
            .is_some_and(|player| player.snapshot().status == PlayerStatus::Playing);
        let interval = if playing {
            PLAYING_REFRESH
        } else {
            IDLE_REFRESH
        };
        // Wake up even when idle so asynchronous transitions (a pause command,
        // end of track) reach MPRIS without waiting for the next input event.
        ctx.request_repaint_after(interval);

        let now = std::time::Instant::now();
        if self
            .mpris_last
            .is_some_and(|last| now.duration_since(last) < interval)
        {
            return;
        }
        self.mpris_last = Some(now);

        service.publish(self.mpris_snapshot());
    }

    #[cfg(target_os = "linux")]
    fn mpris_snapshot(&self) -> crate::mpris::MprisSnapshot {
        use crate::mpris::{LoopMode, MprisSnapshot, MprisStatus};
        use musicwasp_core::PlayerStatus;

        let current_id = self.queue_pos.and_then(|pos| self.queue.get(pos).copied());
        let track = current_id
            .and_then(|id| self.lib.tracks_by_ids(&[id]).ok())
            .and_then(|mut tracks| tracks.pop());
        let player = self.player.as_ref().map(|player| player.snapshot());
        let status = match player.as_ref().map(|snapshot| snapshot.status) {
            Some(PlayerStatus::Playing) => MprisStatus::Playing,
            Some(PlayerStatus::Paused) => MprisStatus::Paused,
            _ => MprisStatus::Stopped,
        };
        let duration = track
            .as_ref()
            .map(|track| track.duration_secs as f64)
            .unwrap_or(0.0);
        let can_next = self
            .queue_pos
            .is_some_and(|pos| pos + 1 < self.queue.len() || self.repeat != RepeatMode::Off);

        MprisSnapshot {
            status,
            track_id: track.as_ref().map(|track| track.id),
            title: track
                .as_ref()
                .map(|track| track.title.clone())
                .unwrap_or_default(),
            artist: track
                .as_ref()
                .map(|track| track.artist.clone())
                .unwrap_or_default(),
            album: track
                .as_ref()
                .map(|track| track.album.clone())
                .unwrap_or_default(),
            duration_secs: duration,
            position_secs: player.as_ref().map_or(0.0, |snapshot| snapshot.position),
            volume: self.volume,
            can_next,
            can_previous: self.queue_pos.is_some(),
            shuffle: self.shuffle,
            loop_mode: match self.repeat {
                RepeatMode::Off => LoopMode::None,
                RepeatMode::All => LoopMode::Playlist,
                RepeatMode::One => LoopMode::Track,
            },
        }
    }

    #[cfg(target_os = "linux")]
    fn poll_mpris(&mut self, ctx: &egui::Context) {
        let mut commands = Vec::new();
        if let Some(service) = &self.mpris {
            while let Some(command) = service.try_recv() {
                commands.push(command);
            }
        }
        for command in commands {
            self.apply_mpris_command(command, ctx);
        }
    }

    #[cfg(target_os = "linux")]
    fn apply_mpris_command(&mut self, command: crate::mpris::MprisCommand, ctx: &egui::Context) {
        use crate::mpris::{LoopMode, MprisCommand};
        use musicwasp_core::PlayerStatus;

        let playing = self
            .player
            .as_ref()
            .is_some_and(|player| player.snapshot().status == PlayerStatus::Playing);
        match command {
            MprisCommand::Play if !playing => self.toggle_pause(),
            MprisCommand::Pause if playing => self.toggle_pause(),
            MprisCommand::PlayPause => self.toggle_pause(),
            MprisCommand::Stop => self.stop(),
            MprisCommand::Next => self.next_track(),
            MprisCommand::Previous => self.prev_track(),
            MprisCommand::Seek(delta) => self.seek_relative(delta),
            MprisCommand::SetPosition(secs) => {
                if let Some(player) = &self.player {
                    player.seek(secs);
                }
            }
            MprisCommand::SetVolume(volume) => {
                self.volume = volume.clamp(0.0, 1.0);
            }
            MprisCommand::SetShuffle(shuffle) => self.shuffle = shuffle,
            MprisCommand::SetLoop(mode) => {
                self.repeat = match mode {
                    LoopMode::None => RepeatMode::Off,
                    LoopMode::Track => RepeatMode::One,
                    LoopMode::Playlist => RepeatMode::All,
                };
            }
            MprisCommand::Raise => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            MprisCommand::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
    }

    // ---- playback ----

    fn play_loaded(&mut self) {
        let Some(pos) = self.queue_pos else { return };
        let Some(id) = self.queue.get(pos).copied() else {
            return;
        };
        let Ok(Some(path)) = self.lib.path_for_id(id) else {
            return;
        };
        if let Some(p) = &self.player {
            p.load_and_play(&path);
        }
        self.selected.clear();
        self.selected.insert(id);
        self.anchor = Some(id);
    }

    pub fn play_from_list(&mut self, index: usize) {
        self.queue = self.tracks.iter().map(|t| t.id).collect();
        if self.queue.is_empty() {
            return;
        }
        self.queue_pos = Some(index.min(self.queue.len() - 1));
        self.play_loaded();
    }

    /// Add tracks to the end of the play queue.
    pub fn queue_append(&mut self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        self.queue.extend_from_slice(ids);
        if let Some(state) = self.queue_state() {
            self.message = format!("added {} track(s) to the queue ({state})", ids.len());
        }
    }

    /// Insert tracks right after the current one; appended when idle.
    pub fn queue_next(&mut self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        queue_insert_next(&mut self.queue, self.queue_pos, ids);
        self.message = format!(
            "{} track(s) will play next{}",
            ids.len(),
            self.queue_state()
                .map(|state| format!(" ({state})"))
                .unwrap_or_default()
        );
    }

    pub fn queue_append_selected(&mut self) {
        let ids = self.selected_in_view();
        self.queue_append(&ids);
    }

    pub fn queue_next_selected(&mut self) {
        let ids = self.selected_in_view();
        self.queue_next(&ids);
    }

    /// Play a specific queue entry.
    pub fn queue_jump(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        self.queue_pos = Some(index);
        self.play_loaded();
    }

    /// Remove an upcoming entry. The playing track is never removed here.
    pub fn queue_remove(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        if self.queue_pos.is_some_and(|pos| index <= pos) {
            return;
        }
        self.queue.remove(index);
    }

    /// Remove every upcoming entry, keeping the current track playing.
    pub fn queue_clear_upcoming(&mut self) {
        match self.queue_pos {
            Some(pos) => self.queue.truncate(pos + 1),
            None => self.queue.clear(),
        }
    }

    /// Move an upcoming entry to a new position (drop before `to`).
    pub fn queue_move(&mut self, from: usize, to: usize) {
        queue_reorder(&mut self.queue, self.queue_pos, from, to);
    }

    fn queue_state(&self) -> Option<String> {
        if self.queue.is_empty() {
            return None;
        }
        Some(match self.queue_pos {
            Some(pos) => format!("{} of {}", pos + 1, self.queue.len()),
            None => format!("{} queued", self.queue.len()),
        })
    }

    pub fn toggle_pause(&mut self) {
        if let Some(p) = &self.player {
            if p.snapshot().status == musicwasp_core::PlayerStatus::Stopped {
                if self.queue_pos.is_none() && !self.queue.is_empty() {
                    // A queue built before playback starts: play its first entry.
                    self.queue_pos = Some(0);
                    self.play_loaded();
                } else if self.queue_pos.is_none() && !self.tracks.is_empty() {
                    self.play_from_list(0);
                } else {
                    self.play_loaded();
                }
            } else {
                p.toggle_pause();
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
    }

    pub fn next_track(&mut self) {
        let Some(pos) = self.queue_pos else { return };
        if self.queue.is_empty() {
            return;
        }
        let next = if self.shuffle && self.queue.len() > 1 {
            // Deterministic-enough shuffle without pulling in rand: step by a
            // co-prime stride so we still visit every track.
            let stride = 7.min(self.queue.len() - 1).max(1);
            (pos + stride) % self.queue.len()
        } else if pos + 1 < self.queue.len() {
            pos + 1
        } else if self.repeat != RepeatMode::Off {
            0
        } else {
            self.queue_pos = None;
            self.stop();
            return;
        };
        self.queue_pos = Some(next);
        self.play_loaded();
    }

    pub fn prev_track(&mut self) {
        let Some(pos) = self.queue_pos else { return };
        if self.queue.is_empty() {
            return;
        }
        let prev = if pos > 0 {
            pos - 1
        } else {
            self.queue.len() - 1
        };
        self.queue_pos = Some(prev);
        self.play_loaded();
    }

    pub fn on_track_finished(&mut self) {
        match self.repeat {
            RepeatMode::One => self.play_loaded(),
            _ => self.next_track(),
        }
    }

    /// Track that would play next, mirroring `next_track` without changing state.
    pub fn upcoming_track(&self) -> Option<Track> {
        let pos = self.queue_pos?;
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        let next = if self.shuffle && len > 1 {
            let stride = 7.min(len - 1).max(1);
            (pos + stride) % len
        } else if pos + 1 < len {
            pos + 1
        } else if self.repeat != RepeatMode::Off {
            0
        } else {
            return None;
        };
        let id = *self.queue.get(next)?;
        if let Some(track) = self.tracks.iter().find(|t| t.id == id) {
            return Some(track.clone());
        }
        let path = self.lib.path_for_id(id).ok().flatten()?;
        self.lib.track_by_path(&path).ok().flatten()
    }

    // ---- keyboard ----

    /// Wait for a key press and bind it to the pending action.
    fn handle_capture(&mut self, ctx: &egui::Context) {
        let Some(action) = self.capture else {
            return;
        };

        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });

        let Some((key, modifiers)) = pressed else {
            return;
        };

        if key == egui::Key::Escape {
            self.capture = None;
            return;
        }

        if key == egui::Key::Delete || key == egui::Key::Backspace {
            self.settings.unbind(action);
        } else {
            self.settings.set(action, Shortcut::new(key, &modifiers));
        }
        self.capture = None;
        self.captured_this_frame = true;
        self.persist_settings();
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.capture.is_some()
            || self.editing.is_some()
            || self.rename.is_some()
            || self.playlist_dialog.is_some()
            || ctx.egui_wants_keyboard_input()
        {
            return;
        }

        for action in Action::ALL {
            if let Some(shortcut) = self.settings.shortcut(action) {
                if shortcut.pressed(ctx) {
                    self.run_action(action);
                }
            }
        }
    }

    pub fn run_action(&mut self, action: Action) {
        match action {
            Action::PlayPause => self.toggle_pause(),
            Action::PlaySelected => {
                if let Some(id) = self.selected_in_view().first().copied() {
                    if let Some(idx) = self.tracks.iter().position(|t| t.id == id) {
                        self.play_from_list(idx);
                    }
                }
            }
            Action::NextTrack => self.next_track(),
            Action::PrevTrack => self.prev_track(),
            Action::SeekForward => self.seek_relative(5.0),
            Action::SeekBackward => self.seek_relative(-5.0),
            Action::Stop => self.stop(),
            Action::VolumeUp => self.nudge_volume(0.05),
            Action::VolumeDown => self.nudge_volume(-0.05),
            Action::EditTags => self.open_editor(),
            Action::Rename => self.start_rename(),
            Action::FocusSearch => self.focus_search = true,
            Action::OpenSettings => self.show_settings = !self.show_settings,
            Action::ToggleEqualizer => self.show_equalizer = !self.show_equalizer,
        }
    }

    /// Jump `delta` seconds from the current position, clamped to the track.
    pub fn seek_relative(&mut self, delta: f64) {
        let Some(player) = &self.player else { return };
        let snapshot = player.snapshot();
        if snapshot.current.is_none() && snapshot.status == musicwasp_core::PlayerStatus::Stopped {
            return;
        }

        player.seek(clamp_seek(snapshot.position, delta, snapshot.duration));
    }

    pub fn nudge_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        if let Some(p) = &self.player {
            p.set_volume(self.volume);
        }
        self.volume_sent = self.volume;
    }

    pub fn persist_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            self.message = format!("could not save settings: {e}");
        }
    }

    /// Restore every setting to its default and apply the ones that are live
    /// (theme, window icon, folder watcher).
    pub fn reset_settings(&mut self, ctx: &egui::Context) {
        self.settings = Settings::default();
        self.capture = None;
        self.watch_dirty = true;
        crate::theme::apply(ctx, self.settings.skin);
        self.refresh_brand_icon(ctx);
        self.persist_settings();
    }

    /// Switch skins immediately and remember the choice.
    pub fn set_skin(&mut self, ctx: &egui::Context, skin: crate::theme::Skin) {
        if self.settings.skin != skin {
            self.settings.skin = skin;
            crate::theme::apply(ctx, skin);
            self.refresh_brand_icon(ctx);
            self.persist_settings();
        }
    }

    /// Swap the toolbar logo and window icon to match the active theme.
    fn refresh_brand_icon(&mut self, ctx: &egui::Context) {
        let light = self.settings.skin.is_light();
        self.logo = crate::icons::logo(ctx, light, 64);
        if let Some(icon) = crate::icons::icon_data(light) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon))));
        }
    }

    /// Push equalizer changes to the audio thread and persist them.
    fn sync_equalizer(&mut self) {
        if self.settings.equalizer == self.eq_applied {
            return;
        }
        let eq = self.settings.equalizer.clamped();
        self.settings.equalizer = eq;
        if let Some(player) = &self.player {
            player.set_equalizer(eq);
        }
        self.eq_applied = eq;
        self.persist_settings();
    }

    /// Push UI volume to the engine when it changed.
    fn sync_volume(&mut self) {
        if (self.volume - self.volume_sent).abs() > f32::EPSILON {
            if let Some(p) = &self.player {
                p.set_volume(self.volume);
            }
            self.volume_sent = self.volume;
        }
    }

    fn update_cover_art(&mut self, ctx: &egui::Context) {
        let current = self.player.as_ref().and_then(|p| p.snapshot().current);
        let Some(path) = current else { return };
        let key = path.to_string_lossy().to_string();
        if self.art_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.art_key = Some(key);
        self.playing_id = self.lib.track_by_path(&path).ok().flatten().map(|t| t.id);

        if let Some(bytes) = musicwasp_core::library::cover_art(&path) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = rgba.into_raw();
                let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                let tex = ctx.load_texture("cover", color, egui::TextureOptions::LINEAR);
                self.art = Some((path.to_string_lossy().to_string(), tex));
                return;
            }
        }
        self.art = None;
    }

    pub fn open_editor(&mut self) {
        let ids = self.selected_in_view();
        if ids.is_empty() {
            return;
        }
        let tracks: Vec<Track> = self
            .tracks
            .iter()
            .filter(|t| ids.contains(&t.id))
            .cloned()
            .collect();

        let (edit, batch) = if tracks.len() == 1 {
            (TagEdit::from_track(&tracks[0]), false)
        } else {
            (common_fields(&tracks), true)
        };

        self.editing = Some(EditState {
            paths: tracks.iter().map(|t| PathBuf::from(&t.path)).collect(),
            edit,
            batch,
            status: None,
        });
    }

    /// Persist the open editor(s) and re-read the files into the library.
    pub fn save_editor(&mut self) {
        let Some(state) = &mut self.editing else {
            return;
        };

        if state.batch && state.edit.is_empty() {
            state.status = Some("nothing to change: fill at least one field".to_string());
            return;
        }

        let mut saved = 0usize;
        let mut failures: Vec<String> = Vec::new();

        for path in &state.paths {
            match musicwasp_core::tags::write_fields(path, &state.edit, state.batch) {
                Ok(()) => {
                    if let Err(e) = self.lib.refresh_file(path) {
                        failures.push(format!("{}: reindex failed: {e}", path.display()));
                        continue;
                    }
                    saved += 1;
                }
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            }
        }

        if failures.is_empty() {
            self.message = format!("saved tags for {saved} track(s)");
            self.editing = None;
            self.art_key = None;
            self.mark_browser_dirty();
        } else {
            state.status = Some(format!(
                "saved {saved}, {} failed: {}",
                failures.len(),
                failures.join("; ")
            ));
        }
    }

    // ---- OS file drag and drop ----

    /// Import files dropped from the OS file manager into the playlist under
    /// the pointer, or into the whole library ("All music").
    fn handle_os_drop(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) =
            ctx.input(|i| (i.raw.dropped_files.clone(), !i.raw.hovered_files.is_empty()));
        let pointer = ctx.input(|i| i.pointer.latest_pos());

        // Remember the row being hovered while files are dragged over us; some
        // platforms deliver the final drop without a fresh pointer position.
        if hovering {
            if let Some(pos) = pointer {
                self.os_drop_hover = self
                    .os_drop_zones
                    .iter()
                    .rev()
                    .find(|zone| zone.rect.contains(pos))
                    .map(|zone| zone.playlist);
            }
        }

        if dropped.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).collect();
        if paths.is_empty() {
            return;
        }
        eprintln!("MusicWasp: received drop of {} path(s)", paths.len());
        // The UI was already drawn this frame; make sure the import result is
        // visible immediately instead of waiting for the next input event.
        ctx.request_repaint();

        let pointer_target = pointer.and_then(|pos| {
            self.os_drop_zones
                .iter()
                .rev()
                .find(|zone| zone.rect.contains(pos))
                .map(|zone| zone.playlist)
        });
        let target = pointer_target.or(self.os_drop_hover.take());
        let Some(target) = target else {
            self.message =
                "drop audio files on a playlist, All music, or the track list".to_string();
            return;
        };

        let indexed = match self.lib.import_paths(&paths) {
            Ok(indexed) => indexed,
            Err(e) => {
                self.message = format!("import failed: {e}");
                return;
            }
        };
        for path in &paths {
            if path.is_dir() {
                self.remember_watch_folder(path);
            }
        }

        let mut ids = Vec::new();
        for path in &indexed {
            if let Ok(Some(track)) = self.lib.track_by_path(path) {
                ids.push(track.id);
            }
        }
        if ids.is_empty() {
            self.message = "no audio files found in that drop".to_string();
            return;
        }

        match target {
            Some(playlist_id) => match self.lib.add_many_to_playlist(playlist_id, &ids) {
                Ok(added) => {
                    let name = self
                        .playlists
                        .iter()
                        .find(|p| p.id == playlist_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "playlist".to_string());
                    self.message = format!("added {added} track(s) to {name}");
                }
                Err(e) => self.message = format!("could not add to playlist: {e}"),
            },
            None => {
                self.message = format!("imported {} track(s) into the library", ids.len());
            }
        }

        self.mark_browser_dirty();
        self.mark_tracks_dirty();
    }

    // ---- rename (F2) ----    /// Open the rename dialog for the focused pane: a track field in the
    /// table, or the playlist currently open in the browser.
    pub fn start_rename(&mut self) {
        if self.rename.is_some() {
            return;
        }

        let target = match self.focus {
            Focus::Tracks => {
                let ids = self.selected_in_view();
                if ids.len() != 1 {
                    self.message = if ids.is_empty() {
                        "select a track, then click the field to rename".to_string()
                    } else {
                        "rename works on one track at a time".to_string()
                    };
                    return;
                }
                RenameTarget::Track {
                    id: ids[0],
                    field: self.rename_field,
                }
            }
            Focus::Browser => match self.view {
                View::Playlist(id) => RenameTarget::Playlist(id),
                View::Smart(id) => RenameTarget::Smart(id),
                View::Library => {
                    self.message = "open a playlist to rename it".to_string();
                    return;
                }
            },
        };

        let text = match &target {
            RenameTarget::Track { id, field } => self
                .tracks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| field.value(t))
                .unwrap_or_default(),
            RenameTarget::Playlist(id) => self
                .playlists
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            RenameTarget::Smart(id) => self
                .smart_playlists
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        };

        self.rename = Some(RenameState {
            target,
            text,
            error: None,
            request_focus: true,
        });
    }

    /// Write the rename back to the tag or the playlist table.
    pub fn commit_rename(&mut self) {
        let Some(state) = self.rename.take() else {
            return;
        };
        let text = state.text.trim().to_string();

        let result = match &state.target {
            RenameTarget::Track { id, field } => {
                let track = self.tracks.iter().find(|t| t.id == *id).cloned();
                match track {
                    None => Err("that track is no longer in this view".to_string()),
                    Some(_)
                        if text.is_empty()
                            && !matches!(field, TrackField::Year | TrackField::TrackNo) =>
                    {
                        Err("the new value cannot be empty".to_string())
                    }
                    Some(track) => {
                        let mut edit = TagEdit::from_track(&track);
                        field.apply(&mut edit, &text).and_then(|()| {
                            let path = PathBuf::from(&track.path);
                            musicwasp_core::tags::write(&path, &edit)
                                .map_err(|e| e.to_string())
                                .and_then(|()| {
                                    self.lib
                                        .refresh_file(&path)
                                        .map_err(|e| format!("reindex failed: {e}"))
                                })
                        })
                    }
                }
            }
            RenameTarget::Playlist(id) => {
                if text.is_empty() {
                    Err("the new name cannot be empty".to_string())
                } else {
                    self.lib
                        .rename_playlist(*id, &text)
                        .map_err(|e| e.to_string())
                }
            }
            RenameTarget::Smart(id) => {
                if text.is_empty() {
                    Err("the new name cannot be empty".to_string())
                } else {
                    self.lib
                        .rename_smart_playlist(*id, &text)
                        .map_err(|e| e.to_string())
                }
            }
        };

        match result {
            Ok(()) => {
                self.message = match &state.target {
                    RenameTarget::Track { field, .. } => {
                        format!("{} updated", field.label().to_lowercase())
                    }
                    _ => "playlist renamed".to_string(),
                };
                self.mark_browser_dirty();
                self.mark_tracks_dirty();
            }
            Err(error) => {
                self.rename = Some(RenameState {
                    target: state.target,
                    text: state.text,
                    error: Some(error),
                    request_focus: false,
                });
            }
        }
    }

    // ---- auto-tag by album ----

    /// Tracks the auto-tag dialog operates on: the selection, else the view.
    pub fn autotag_targets(&self) -> Vec<Track> {
        let selected: BTreeSet<i64> = self.selected_in_view().into_iter().collect();
        let pool: Vec<Track> = if selected.is_empty() {
            self.tracks.clone()
        } else {
            self.tracks
                .iter()
                .filter(|t| selected.contains(&t.id))
                .cloned()
                .collect()
        };
        pool
    }

    pub fn open_autotag(&mut self) {
        let targets = self.autotag_targets();
        if targets.is_empty() {
            self.message = "select the album's tracks first".to_string();
            return;
        }
        let first = &targets[0];
        self.autotag = Some(AutoTagState::new(
            first.artist.clone(),
            first.album.clone(),
            targets,
        ));
    }

    pub fn start_autotag_search(&mut self) {
        let Some(state) = &mut self.autotag else {
            return;
        };
        let (artist, album) = (
            state.artist.trim().to_string(),
            state.album.trim().to_string(),
        );
        if album.is_empty() {
            state.stage = AutoTagStage::Done("enter an album name first".to_string());
            return;
        }
        state.stage = AutoTagStage::Searching;
        let tx = state.tx.clone();
        std::thread::spawn(move || {
            let result = autotag::search(&artist, &album).map_err(|e| format!("{e:#}"));
            let _ = tx.send(AutoTagMsg::SearchDone(result));
        });
    }

    /// Fetch the selected candidate's track list so a single file can be
    /// matched to the right song instead of the first one.
    pub fn load_autotag_release(&mut self) {
        let Some(state) = &mut self.autotag else {
            return;
        };
        let AutoTagStage::Results(candidates) = &state.stage else {
            return;
        };
        let Some(candidate) = candidates.get(state.selected) else {
            return;
        };
        let release_id = candidate.release_id.clone();

        if state
            .release
            .as_ref()
            .is_some_and(|release| release.id == release_id)
        {
            return;
        }
        if state.loading_release && state.release_id.as_deref() == Some(release_id.as_str()) {
            return;
        }

        state.release_id = Some(release_id.clone());
        state.release = None;
        state.release_error = None;
        state.loading_release = true;
        let tx = state.tx.clone();
        std::thread::spawn(move || {
            let result = autotag::fetch(&release_id).map_err(|e| format!("{e:#}"));
            let _ = tx.send(AutoTagMsg::ReleaseLoaded { release_id, result });
        });
    }

    pub fn start_autotag_apply(&mut self) {
        let Some(state) = &self.autotag else {
            return;
        };
        let AutoTagStage::Results(candidates) = &state.stage else {
            return;
        };
        let Some(candidate) = candidates.get(state.selected).cloned() else {
            return;
        };
        let Some(release) = state
            .release
            .clone()
            .filter(|release| release.id == candidate.release_id)
        else {
            return;
        };

        let matches = mapping_matches(&state.mappings, &release);
        if matches.is_empty() {
            return;
        }

        let embed_art = state.embed_art;
        let tx = state.tx.clone();
        if let Some(state) = &mut self.autotag {
            state.stage = AutoTagStage::Applying;
        }

        std::thread::spawn(move || {
            let cover = if embed_art {
                autotag::front_cover(&candidate.release_id).ok().flatten()
            } else {
                None
            };
            let (written, errors) = autotag::apply(&release, &matches, cover.as_ref());
            let paths = matches.iter().map(|m| m.path.clone()).collect();
            let _ = tx.send(AutoTagMsg::Applied {
                written,
                errors,
                paths,
                cover: cover.is_some(),
                cover_requested: embed_art,
            });
        });
    }

    fn poll_autotag(&mut self) {
        let mut messages = Vec::new();
        if let Some(state) = &self.autotag {
            while let Ok(msg) = state.rx.try_recv() {
                messages.push(msg);
            }
        }
        if messages.is_empty() {
            return;
        }

        for msg in messages {
            match msg {
                AutoTagMsg::SearchDone(Ok(candidates)) => {
                    let local_count = self.autotag_targets().len();
                    if let Some(state) = &mut self.autotag {
                        let mut ranked = candidates;
                        ranked.sort_by_key(|c| -autotag::relevance(c, local_count));
                        state.selected = 0;
                        state.stage = AutoTagStage::Results(ranked);
                        state.release = None;
                        state.release_id = None;
                        state.release_error = None;
                        state.loading_release = false;
                        state.mappings.clear();
                    }
                    // Load the top candidate's tracks so a single-file tag can
                    // show a song picker right away.
                    self.load_autotag_release();
                }
                AutoTagMsg::SearchDone(Err(e)) => {
                    if let Some(state) = &mut self.autotag {
                        state.stage = AutoTagStage::Done(format!("search failed: {e}"));
                    }
                }
                AutoTagMsg::ReleaseLoaded { release_id, result } => {
                    if let Some(state) = &mut self.autotag {
                        if state.release_id.as_deref() != Some(release_id.as_str()) {
                            continue;
                        }
                        state.loading_release = false;
                        match result {
                            Ok(release) => {
                                state.mappings = suggested_mappings(&state.targets, &release);
                                state.release_error = None;
                                state.release = Some(release);
                            }
                            Err(e) => {
                                state.release = None;
                                state.mappings.clear();
                                state.release_error = Some(e);
                            }
                        }
                    }
                }
                AutoTagMsg::Applied {
                    written,
                    errors,
                    paths,
                    cover,
                    cover_requested,
                } => {
                    for path in &paths {
                        if let Err(e) = self.lib.refresh_file(path) {
                            self.message = format!("reindex failed: {e}");
                        }
                    }
                    // The file changed on disk: drop the cached cover so the
                    // new embedded art is loaded on the next frame.
                    self.art_key = None;
                    self.mark_browser_dirty();
                    self.mark_tracks_dirty();

                    let mut summary = format!("tagged {written} file(s)");
                    if cover {
                        summary.push_str(", cover embedded");
                    } else if cover_requested {
                        summary.push_str(", no cover found");
                    }
                    if !errors.is_empty() {
                        summary.push_str(&format!(", {} failed", errors.len()));
                        if let Some(first) = errors.first() {
                            summary.push_str(&format!(" ({first})"));
                        }
                    }
                    if let Some(state) = &mut self.autotag {
                        state.stage = AutoTagStage::Done(summary.clone());
                    }
                    self.message = summary;
                }
            }
        }
    }

    pub fn open_picker(&mut self) {
        let start = PathBuf::from(self.scan_path.trim());
        self.picker = Some(FolderPicker::new(start));
    }

    fn scan(&mut self) {
        let path = std::path::PathBuf::from(self.scan_path.trim());
        match self.lib.scan(&path, false) {
            Ok(n) => {
                self.message = format!("indexed {n} tracks from {}", path.display());
                self.remember_watch_folder(&path);
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("scan failed: {e}"),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.capture_window(&ctx);
        if self.watch_dirty {
            self.restart_watcher(&ctx);
            self.watch_dirty = false;
        }
        self.poll_watcher(&ctx);
        #[cfg(target_os = "linux")]
        self.poll_mpris(&ctx);
        self.poll_autotag();
        self.handle_capture(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_screenshot_capture(&ctx);

        if let Some(p) = &self.player {
            if p.take_finished() {
                self.on_track_finished();
            }
        }
        self.sync_volume();
        self.sync_equalizer();
        self.update_cover_art(&ctx);

        if self.dirty_browser {
            self.refresh_browser();
        }
        if self.dirty_tracks {
            self.refresh_tracks();
        }

        crate::ui::toolbar(self, root);
        if self.plain_view {
            self.os_drop_zones.clear();
            crate::ui::now_playing(self, root);
        } else {
            crate::ui::browser(self, root);
            crate::ui::transport(self, root);
            crate::ui::queue_panel(self, root);
            crate::ui::tracklist(self, root);
        }
        // Handle OS file drops after the browser has registered this frame's
        // playlist rows, so the pointer maps to the row under it.
        self.handle_os_drop(&ctx);
        crate::ui::tag_editor(self, &ctx);
        crate::ui::playlist_dialog(self, &ctx);
        crate::ui::smart_dialog(self, &ctx);
        crate::ui::settings_window(self, &ctx);
        crate::ui::hygiene_window(self, &ctx);
        crate::ui::folder_picker(self, &ctx);
        crate::ui::autotag_window(self, &ctx);
        crate::ui::rename_window(self, &ctx);
        crate::equalizer::window(self, &ctx);

        // A drag released outside any drop target must not linger as a payload.
        if egui::DragAndDrop::has_any_payload(&ctx) && ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(&ctx);
        }

        if let Some(state) = &self.autotag {
            if state.loading_release
                || matches!(
                    state.stage,
                    AutoTagStage::Searching | AutoTagStage::Applying
                )
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }

        if let Some(p) = &self.player {
            if p.snapshot().status == musicwasp_core::PlayerStatus::Playing {
                // Repaint while playing so the position slider stays live.
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }

        #[cfg(target_os = "linux")]
        self.publish_mpris(&ctx);

        self.persist_session(false);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_session(true);
    }
}

impl App {
    pub fn scan_clicked(&mut self) {
        self.scan();
    }

    fn handle_screenshot_capture(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot_path.clone() else {
            return;
        };
        self.frames += 1;

        // Debug hook: MUSICWASP_AUTOPLAY=1 starts the first track (muted).
        if self.frames == 15
            && std::env::var("MUSICWASP_AUTOPLAY").is_ok()
            && !self.tracks.is_empty()
        {
            self.volume = 0.0;
            self.play_from_list(0);
        }

        // Debug hook: MUSICWASP_SETTINGS=1 opens the settings panel;
        // MUSICWASP_SETTINGS_TAB=keybindings|about picks a tab.
        if self.frames == 20 && std::env::var("MUSICWASP_SETTINGS").is_ok() {
            self.show_settings = true;
            self.settings_tab = match std::env::var("MUSICWASP_SETTINGS_TAB").as_deref() {
                Ok("keybindings") => SettingsTab::Keybindings,
                Ok("about") => SettingsTab::About,
                _ => SettingsTab::General,
            };
        }

        // Debug hook: MUSICWASP_EDITOR=1 opens the tag editor for the first
        // track; MUSICWASP_EDITOR=multi selects everything first.
        if self.frames == 20 {
            match std::env::var("MUSICWASP_EDITOR").as_deref() {
                Ok("multi") => {
                    self.select_all_in_view();
                    self.open_editor();
                }
                Ok(_) if !self.tracks.is_empty() => {
                    self.selected.clear();
                    self.selected.insert(self.tracks[0].id);
                    self.anchor = Some(self.tracks[0].id);
                    self.open_editor();
                }
                _ => {}
            }
        }

        // Debug hook: MUSICWASP_RENAME=1 opens rename for the first track title.
        if self.frames == 20
            && std::env::var("MUSICWASP_RENAME").is_ok()
            && self.rename.is_none()
            && !self.tracks.is_empty()
        {
            let id = self.tracks[0].id;
            self.selected.clear();
            self.selected.insert(id);
            self.anchor = Some(id);
            self.rename_field = TrackField::Title;
            self.focus = Focus::Tracks;
            self.start_rename();
        }

        // Debug hook: MUSICWASP_AUTOTAG=1 opens the dialog and searches;
        // MUSICWASP_AUTOTAG=one tags just the first track.
        if self.frames == 20 && !self.tracks.is_empty() {
            match std::env::var("MUSICWASP_AUTOTAG").as_deref() {
                Ok("one") => {
                    let id = self.tracks[0].id;
                    self.selected.clear();
                    self.selected.insert(id);
                    self.anchor = Some(id);
                    self.open_autotag();
                    self.start_autotag_search();
                }
                Ok(_) => {
                    self.select_all_in_view();
                    self.open_autotag();
                    self.start_autotag_search();
                }
                Err(_) => {}
            }
        }

        // Debug hook: MUSICWASP_AUTOTAG_APPLY=1 applies the loaded candidate
        // (paired with MUSICWASP_AUTOTAG=one).
        if std::env::var("MUSICWASP_AUTOTAG_APPLY").is_ok() && self.frames > 20 {
            let ready = self.autotag.as_ref().is_some_and(|state| {
                matches!(state.stage, AutoTagStage::Results(_))
                    && state.release.is_some()
                    && !state.loading_release
            });
            if ready {
                self.start_autotag_apply();
            }
        }

        // Debug hook: MUSICWASP_SMART=1 opens the smart playlist editor.
        if self.frames == 20 && std::env::var("MUSICWASP_SMART").is_ok() {
            self.open_smart_dialog();
        }

        // Debug hook: MUSICWASP_PLAYLIST=1 (or =smart) opens a playlist view.
        if self.frames == 20 {
            match std::env::var("MUSICWASP_PLAYLIST").as_deref() {
                Ok("smart") => {
                    if let Some(first) = self.smart_playlists.first() {
                        self.view = View::Smart(first.id);
                        self.mark_tracks_dirty();
                    }
                }
                Ok(_) => {
                    if let Some(first) = self.playlists.first() {
                        self.view = View::Playlist(first.id);
                        self.mark_tracks_dirty();
                    }
                }
                Err(_) => {}
            }
        }

        // Debug hook: MUSICWASP_PICKER=1 opens the folder chooser.
        if self.frames == 20 && std::env::var("MUSICWASP_PICKER").is_ok() {
            self.open_picker();
        }

        // Debug hook: MUSICWASP_EQ=1 opens the equalizer.
        if self.frames == 20 && std::env::var("MUSICWASP_EQ").is_ok() {
            self.show_equalizer = true;
        }

        // Debug hook: MUSICWASP_HYGIENE=1 opens the library hygiene window.
        if self.frames == 20 && std::env::var("MUSICWASP_HYGIENE").is_ok() {
            self.open_hygiene();
        }

        let shot_frame = std::env::var("MUSICWASP_SCREENSHOT_FRAME")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(40);

        if self.frames == shot_frame && !self.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.shot_requested = true;
            return;
        }

        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });

        if let Some(img) = image {
            let [w, h] = img.size;
            if let Some(buf) = image::RgbaImage::from_raw(
                w as u32,
                h as u32,
                img.pixels.iter().flat_map(|c| c.to_array()).collect(),
            ) {
                let _ = buf.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        ctx.request_repaint();
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Relative seek that never goes before the start or past the end of a track.
pub fn clamp_seek(position: f64, delta: f64, duration: f64) -> f64 {
    let target = (position + delta).max(0.0);
    if duration > 0.0 {
        target.min((duration - 0.05).max(0.0))
    } else {
        target
    }
}

pub fn fmt_size(bytes: u64) -> String {
    const MB: f64 = 1_048_576.0;
    if bytes as f64 >= 1024.0 * MB {
        format!("{:.1} GB", bytes as f64 / (1024.0 * MB))
    } else {
        format!("{:.1} MB", bytes as f64 / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_click, best_release_track, clamp_seek, common_fields, list_subdirs, mapping_matches,
        queue_insert_next, queue_reorder, remove_tracks_from_queue, restore_queue,
        suggested_mappings, TrackField,
    };
    use musicwasp_core::autotag::{Release, ReleaseTrack};
    use musicwasp_core::{TagEdit, Track};
    use std::collections::BTreeSet;

    #[test]
    fn renaming_numeric_fields_validates_ranges() {
        let mut edit = TagEdit::default();
        TrackField::Year.apply(&mut edit, "1999").unwrap();
        assert_eq!(edit.year, Some(1999));
        TrackField::Year.apply(&mut edit, "").unwrap();
        assert_eq!(edit.year, None);
        assert!(TrackField::Year.apply(&mut edit, "nineteen").is_err());

        TrackField::TrackNo.apply(&mut edit, "1000").unwrap_err();
        TrackField::TrackNo.apply(&mut edit, "7").unwrap();
        assert_eq!(edit.track_no, Some(7));

        TrackField::Title.apply(&mut edit, "  Song  ").unwrap();
        assert_eq!(edit.title, "Song");
    }

    #[test]
    fn relative_seek_stays_inside_the_track() {
        assert_eq!(clamp_seek(10.0, 5.0, 60.0), 15.0);
        assert_eq!(clamp_seek(10.0, -5.0, 60.0), 5.0);
        // Cannot rewind before the start.
        assert_eq!(clamp_seek(2.0, -5.0, 60.0), 0.0);
        // Forward past the end lands just short of it.
        assert!((clamp_seek(58.0, 5.0, 60.0) - 59.95).abs() < 1e-9);
        // Unknown duration: clamp only at zero.
        assert_eq!(clamp_seek(2.0, 5.0, 0.0), 7.0);
    }

    fn selection(ids: &[i64]) -> BTreeSet<i64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn plain_click_replaces_the_selection() {
        let view = [10, 20, 30];
        let mut selected = selection(&[10, 20]);
        let mut anchor = Some(10);

        apply_click(&mut selected, &mut anchor, &view, 2, false, false);
        assert_eq!(selected, selection(&[30]));
        assert_eq!(anchor, Some(30));
    }

    #[test]
    fn ctrl_click_toggles_rows() {
        let view = [10, 20, 30];
        let mut selected = selection(&[10]);
        let mut anchor = None;

        apply_click(&mut selected, &mut anchor, &view, 1, true, false);
        assert_eq!(selected, selection(&[10, 20]));
        assert_eq!(anchor, Some(20));

        // Clicking the same row again removes it.
        apply_click(&mut selected, &mut anchor, &view, 1, true, false);
        assert_eq!(selected, selection(&[10]));
    }

    #[test]
    fn shift_click_selects_a_range_from_the_anchor() {
        let view = [10, 20, 30, 40];
        let mut selected = selection(&[]);
        let mut anchor = None;

        apply_click(&mut selected, &mut anchor, &view, 1, false, false);
        apply_click(&mut selected, &mut anchor, &view, 3, false, true);
        assert_eq!(selected, selection(&[20, 30, 40]));

        // Backwards range too.
        apply_click(&mut selected, &mut anchor, &view, 0, false, true);
        assert_eq!(selected, selection(&[10, 20]));
    }

    #[test]
    fn shift_click_without_anchor_falls_back_to_single_select() {
        let view = [10, 20];
        let mut selected = selection(&[]);
        let mut anchor = None;
        apply_click(&mut selected, &mut anchor, &view, 1, false, true);
        assert_eq!(selected, selection(&[20]));
        assert_eq!(anchor, Some(20));
    }

    #[test]
    fn out_of_range_clicks_are_ignored() {
        let view = [10];
        let mut selected = selection(&[10]);
        let mut anchor = Some(10);
        apply_click(&mut selected, &mut anchor, &view, 7, false, false);
        assert_eq!(selected, selection(&[10]));
    }

    #[test]
    fn queue_inserts_after_the_current_track_and_reorders_only_up_next() {
        let mut queue = vec![1, 2, 3];
        queue_insert_next(&mut queue, Some(0), &[8, 9]);
        assert_eq!(queue, vec![1, 8, 9, 2, 3]);

        queue_insert_next(&mut queue, None, &[7]);
        assert_eq!(queue, vec![1, 8, 9, 2, 3, 7]);

        // Reorder upcoming entries.
        assert!(queue_reorder(&mut queue, Some(0), 4, 1));
        assert_eq!(queue, vec![1, 3, 8, 9, 2, 7]);

        // Move to the very end.
        let end = queue.len();
        assert!(queue_reorder(&mut queue, Some(0), 2, end));
        assert_eq!(queue, vec![1, 3, 9, 2, 7, 8]);

        // Never move the playing entry or anything before it.
        assert!(!queue_reorder(&mut queue, Some(0), 0, 3));
        assert!(!queue_reorder(&mut queue, Some(2), 1, 4));
        assert_eq!(queue, vec![1, 3, 9, 2, 7, 8]);
    }

    #[test]
    fn restored_queue_drops_deleted_tracks_and_keeps_the_playing_slot() {
        let valid = selection(&[1, 3, 9]);
        let (queue, pos) = restore_queue(&[1, 2, 3, 9, 1], Some(3), &valid);
        assert_eq!(queue, vec![1, 3, 9, 1]);
        assert_eq!(pos, Some(2), "the third entry is still the current one");

        // If the current track was deleted, do not start some other track.
        let (queue, pos) = restore_queue(&[1, 2, 3], Some(1), &valid);
        assert_eq!(queue, vec![1, 3]);
        assert_eq!(pos, None);

        // Duplicate ids keep their position.
        let (queue, pos) = restore_queue(&[9, 1, 9], Some(2), &valid);
        assert_eq!(queue, vec![9, 1, 9]);
        assert_eq!(pos, Some(2));
    }

    #[test]
    fn removing_library_tracks_stops_only_if_the_current_entry_is_gone() {
        let removed = selection(&[2]);

        let mut queue = vec![1, 2, 3, 4];
        let (pos, current_removed) = remove_tracks_from_queue(&mut queue, Some(2), &removed);
        assert_eq!(queue, vec![1, 3, 4]);
        assert_eq!(pos, Some(1));
        assert!(!current_removed);

        let mut queue = vec![1, 2, 4];
        let (pos, current_removed) = remove_tracks_from_queue(&mut queue, Some(1), &removed);
        assert_eq!(queue, vec![1, 4]);
        assert_eq!(pos, None);
        assert!(current_removed);
    }

    fn track(title: &str, artist: &str, album: &str, year: Option<i32>) -> Track {
        Track {
            id: 0,
            path: String::new(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: String::new(),
            genre: String::new(),
            year,
            track_no: None,
            disc_no: None,
            duration_secs: 0,
            sample_rate: 0,
            bitrate: 0,
            format: String::new(),
        }
    }

    fn release_track(disc: u32, number: u32, title: &str) -> ReleaseTrack {
        ReleaseTrack {
            disc,
            number,
            title: title.into(),
            artist: "Artist".into(),
            length_ms: Some(200_000),
        }
    }

    #[test]
    fn single_file_tagging_picks_the_right_release_track() {
        let release = Release {
            id: "r".into(),
            title: "Album".into(),
            artist: "Artist".into(),
            album_artist: "Artist".into(),
            year: None,
            genre: None,
            tracks: vec![
                release_track(1, 1, "First Song"),
                release_track(1, 2, "Second Song"),
                release_track(2, 1, "Third Song"),
            ],
            has_front_cover: false,
        };

        let mut local = track("Wrong local title", "Artist", "Album", None);
        local.track_no = Some(2);
        local.disc_no = Some(1);
        assert_eq!(best_release_track(&local, &release), 1);

        // No usable number: fall back to an exact title match.
        local.track_no = None;
        local.title = "Third Song".into();
        assert_eq!(best_release_track(&local, &release), 2);

        // Nothing matches: closest duration, here all equal, so the first.
        local.title = "Unknown".into();
        assert_eq!(best_release_track(&local, &release), 0);
    }

    #[test]
    fn album_preview_maps_by_track_number_and_exposes_diffs() {
        let release = Release {
            id: "r".into(),
            title: "Album".into(),
            artist: "Artist".into(),
            album_artist: "Artist".into(),
            year: Some(2000),
            genre: None,
            tracks: vec![
                release_track(1, 1, "First Song"),
                release_track(1, 2, "Second Song"),
            ],
            has_front_cover: false,
        };

        let mut second = track("Wrong two", "Artist", "Album", None);
        second.track_no = Some(2);
        let mut first = track("Wrong one", "Artist", "Album", None);
        first.track_no = Some(1);

        let mut mappings = suggested_mappings(&[second, first], &release);
        assert_eq!(mappings[0].release_index, Some(1));
        assert_eq!(mappings[1].release_index, Some(0));

        let changes = mappings[1].changes(&release);
        assert!(changes
            .iter()
            .any(|(field, _, new)| *field == "Title" && new == "First Song"));

        // A manual override changes what gets written; skip removes the row.
        mappings[0].release_index = Some(0);
        mappings[1].release_index = None;
        let matches = mapping_matches(&mappings, &release);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].track.title, "First Song");
    }

    #[test]
    fn batch_prefill_keeps_shared_fields_and_blanks_differing_ones() {
        let tracks = [
            track("One", "Same Artist", "Album A", Some(2001)),
            track("Two", "Same Artist", "Album B", Some(2002)),
        ];
        let common = common_fields(&tracks);

        assert_eq!(common.artist, "Same Artist", "shared value is prefilled");
        assert_eq!(common.title, "", "differing values stay blank");
        assert_eq!(common.album, "");
        assert_eq!(common.year, None);
        assert!(!common.is_empty());
    }

    #[test]
    fn folder_listing_hides_dot_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Music")).unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join("file.mp3"), b"x").unwrap();

        let visible = list_subdirs(dir.path(), false).unwrap();
        assert_eq!(visible.len(), 1);
        assert!(visible[0].ends_with("Music"));

        let all = list_subdirs(dir.path(), true).unwrap();
        assert_eq!(all.len(), 2);
    }
}
