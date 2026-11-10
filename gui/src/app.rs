use crate::debug_hooks as debug_env;
use crate::i18n::{tr, Language};
use crate::settings::{
    Action, Session, SessionView, Settings, Shortcut, TrackColumn, WindowGeometry,
};
use musicwasp_core::autotag::{self, Candidate, Cover, LocalTrack, Release};
use musicwasp_core::dsp::Equalizer;
use musicwasp_core::lyrics::Lyrics;
use musicwasp_core::smart::{Definition, Field, Op, Rule, SmartPlaylist};
use musicwasp_core::tags::TagSnapshot;
use musicwasp_core::{
    AlbumGroup, Filter, Library, Player, Playlist, PlaylistFolder, ReconcilePlan, SavedSearch,
    ScanProgress, ScanReport, SortKey, TagEdit, Track,
};
use notify::Watcher as _;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

mod debug_hooks;
mod podcast;
mod stream;
mod visualizer;

pub use podcast::{PodcastMsg, PodcastState};
pub use stream::{RadioState, StreamInfo, StreamKind, StreamState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Playlist(i64),
    Smart(i64),
    Podcasts,
    Radio,
}

impl From<View> for SessionView {
    fn from(view: View) -> Self {
        match view {
            View::Library => SessionView::Library,
            View::Playlist(id) => SessionView::Playlist(id),
            View::Smart(id) => SessionView::Smart(id),
            View::Podcasts => SessionView::Podcasts,
            View::Radio => SessionView::Radio,
        }
    }
}

impl From<SessionView> for View {
    fn from(view: SessionView) -> Self {
        match view {
            SessionView::Library => View::Library,
            SessionView::Playlist(id) => View::Playlist(id),
            SessionView::Smart(id) => View::Smart(id),
            SessionView::Podcasts => View::Podcasts,
            SessionView::Radio => View::Radio,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    LookAndFeel,
    Tracks,
    Visualizer,
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

/// Drag payload for moving a playlist into a folder.
#[derive(Debug, Clone, Copy)]
pub struct DragPlaylist {
    pub id: i64,
    /// True for smart playlists, which are stored in another table.
    pub smart: bool,
}

/// Drag payload for reordering the play queue.
#[derive(Debug, Clone, Copy)]
pub struct QueueDrag {
    pub index: usize,
}

/// Editing state for a new or existing smart playlist.
pub struct SmartDraft {
    pub name: String,
    pub definition: Definition,
    pub status: Option<String>,
    /// Folder for a new playlist; existing playlists keep their folder.
    pub folder: Option<i64>,
    /// `Some(id)` edits an existing playlist; `None` creates one.
    pub editing: Option<i64>,
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

/// Case style used by the batch tag actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchCase {
    #[default]
    Title,
    Lower,
    Upper,
}

impl BatchCase {
    pub const ALL: [Self; 3] = [Self::Title, Self::Lower, Self::Upper];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title Case",
            Self::Lower => "lowercase",
            Self::Upper => "UPPERCASE",
        }
    }
}

/// A tag field targeted by clear/copy batch actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchField {
    #[default]
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    TrackNo,
    DiscNo,
}

impl BatchField {
    pub const ALL: [Self; 8] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Genre,
        Self::Year,
        Self::TrackNo,
        Self::DiscNo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => tr("column.title"),
            Self::Artist => tr("column.artist"),
            Self::Album => "Album",
            Self::AlbumArtist => "Album artist",
            Self::Genre => tr("column.genre"),
            Self::Year => tr("column.year"),
            Self::TrackNo => "Track no",
            Self::DiscNo => "Disc no",
        }
    }

    fn value(self, edit: &TagEdit) -> String {
        match self {
            Self::Title => edit.title.clone(),
            Self::Artist => edit.artist.clone(),
            Self::Album => edit.album.clone(),
            Self::AlbumArtist => edit.album_artist.clone(),
            Self::Genre => edit.genre.clone(),
            Self::Year => edit.year.map(|value| value.to_string()).unwrap_or_default(),
            Self::TrackNo => edit
                .track_no
                .map(|value| value.to_string())
                .unwrap_or_default(),
            Self::DiscNo => edit
                .disc_no
                .map(|value| value.to_string())
                .unwrap_or_default(),
        }
    }

    fn set(self, edit: &mut TagEdit, value: &str) {
        match self {
            Self::Title => edit.title = value.to_string(),
            Self::Artist => edit.artist = value.to_string(),
            Self::Album => edit.album = value.to_string(),
            Self::AlbumArtist => edit.album_artist = value.to_string(),
            Self::Genre => edit.genre = value.to_string(),
            Self::Year => edit.year = value.trim().parse().ok(),
            Self::TrackNo => edit.track_no = value.trim().parse().ok(),
            Self::DiscNo => edit.disc_no = value.trim().parse().ok(),
        }
    }

    fn clear(self, edit: &mut TagEdit) {
        self.set(edit, "");
    }
}

/// Immediate batch tag transformations; unlike the shared edit fields these
/// need to read and write each file individually.
#[derive(Debug, Clone, Copy)]
pub enum BatchAction {
    AutoNumber,
    Case(BatchCase),
    Clear(BatchField),
    Copy(BatchField),
}

impl BatchAction {
    fn label(self) -> &'static str {
        match self {
            Self::AutoNumber => "auto-number",
            Self::Case(_) => "normalize case",
            Self::Clear(_) => "clear field",
            Self::Copy(_) => "copy field",
        }
    }
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
            TrackField::Title => tr("column.title"),
            TrackField::Artist => tr("column.artist"),
            TrackField::Album => "Album",
            TrackField::Genre => tr("column.genre"),
            TrackField::Year => tr("column.year"),
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
    Folder(i64),
}

impl RenameTarget {
    pub fn title(&self) -> String {
        match self {
            RenameTarget::Track { field, .. } => format!("Rename {}", field.label().to_lowercase()),
            RenameTarget::Playlist(_) => "Rename playlist".to_string(),
            RenameTarget::Smart(_) => "Rename smart playlist".to_string(),
            RenameTarget::Folder(_) => "Rename folder".to_string(),
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
/// `playlist` is `None` for the whole library (tr("ui.all_music")).
#[derive(Debug, Clone, Copy)]
pub struct OsDropZone {
    pub playlist: Option<i64>,
    pub rect: egui::Rect,
}

// ---------------------------------------------------------------- auto-tag

#[derive(Debug)]
pub enum AutoTagMsg {
    SearchDone(Result<Vec<Candidate>, String>),
    Fingerprinted(Result<Vec<Candidate>, String>),
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
    Failed(String),
}

pub enum AutoTagStage {
    Editing,
    Searching,
    Fingerprinting,
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
        compare(
            tr("column.title"),
            self.track.title.clone(),
            track.title.clone(),
        );
        compare(
            tr("column.artist"),
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
            tr("column.year"),
            self.track.year.map(|y| y.to_string()).unwrap_or_default(),
            release
                .year
                .map(|y| y.to_string())
                .or_else(|| self.track.year.map(|y| y.to_string()))
                .unwrap_or_default(),
        );
        compare(
            tr("column.genre"),
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
/// Restore-library dialog state.
pub struct RestoreDraft {
    pub path: String,
    pub backups: Vec<musicwasp_core::backup::BackupFile>,
    pub confirm: bool,
    pub status: Option<String>,
}

/// Live skin colour editor state.
pub struct SkinEditorState {
    pub skin: crate::theme::Skin,
    pub palette: crate::theme::Palette,
    pub status: Option<String>,
}

/// File organizer window: pattern, dry-run plan and confirmation state.
pub struct OrganizeState {
    pub pattern: String,
    pub root: String,
    /// Restrict the next preview to the tracks selected in the current view.
    pub selected_only: bool,
    pub plan: Option<musicwasp_core::organize::OrganizePlan>,
    pub pending: bool,
    pub status: Option<String>,
}

pub struct HygieneState {
    pub missing: Vec<Track>,
    pub duplicate_groups: Vec<Vec<Track>>,
    pub selected: BTreeSet<i64>,
    /// Ids awaiting confirmation before they are removed from the database.
    pub pending: Option<Vec<i64>>,
    /// Dry-run rename preview; `None` until the user asks for one.
    pub reconcile: Option<ReconcilePlan>,
    /// Missing-row ids ticked in the rename preview.
    pub rename_selected: BTreeSet<i64>,
    /// Rename candidates awaiting confirmation.
    pub pending_renames: Option<Vec<i64>>,
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

/// New-folder dialog state.
#[derive(Debug, Clone)]
pub struct FolderDraft {
    pub name: String,
    /// Folder that will contain the new folder; `None` creates a root folder.
    pub parent_id: Option<i64>,
    pub status: Option<String>,
}

/// What a playlist transfer dialog is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M3uMode {
    Import,
    /// Export the manual playlist with this id.
    Export(i64),
    /// Export the evaluated smart playlist with this id.
    ExportSmart(i64),
}

/// Path dialog for importing and exporting M3U playlists.
#[derive(Debug, Clone)]
pub struct M3uDraft {
    pub mode: M3uMode,
    pub path: String,
    /// Optional name override when importing.
    pub name: String,
    /// Destination folder when importing.
    pub folder_id: Option<i64>,
    pub status: Option<String>,
}

/// Editing state for a named search.
#[derive(Debug, Clone)]
pub struct SavedSearchDraft {
    /// `Some(id)` edits an existing saved search.
    pub id: Option<i64>,
    pub name: String,
    pub query: String,
    pub status: Option<String>,
}

/// Small in-app image chooser for replacing album covers.
pub struct CoverPickerState {
    pub album: String,
    pub artist: String,
    pub current: PathBuf,
    pub dirs: Vec<PathBuf>,
    pub images: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
    pub error: Option<String>,
}

impl CoverPickerState {
    pub fn new(album: String, artist: String, start: PathBuf) -> Self {
        let mut picker = Self {
            album,
            artist,
            current: start,
            dirs: Vec::new(),
            images: Vec::new(),
            selected: None,
            error: None,
        };
        if !picker.current.is_dir() {
            picker.current = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        }
        picker.reload();
        picker
    }

    pub fn reload(&mut self) {
        self.dirs.clear();
        self.images.clear();
        self.error = None;
        match std::fs::read_dir(&self.current) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        self.dirs.push(path);
                    } else if is_image_path(&path) {
                        self.images.push(path);
                    }
                }
                self.dirs.sort();
                self.images.sort();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" | "tif" | "tiff"
            )
        })
}

/// Turn a playlist name into a portable file name for M3U export.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "playlist".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Result of a background Cover Art Archive lookup.
struct CoverMsg {
    album: String,
    artist: String,
    result: Result<Option<Cover>, String>,
}

/// Messages from the background indexing worker.
enum ScanMsg {
    Progress(ScanProgress),
    Finished(std::result::Result<ScanReport, String>),
}

/// Background scan state. The worker owns its own SQLite connection.
struct ScanState {
    root: PathBuf,
    progress: ScanProgress,
    cancel: Arc<AtomicBool>,
    rx: Receiver<ScanMsg>,
    last_refresh: std::time::Instant,
}

/// A play currently being considered for ListenBrainz submission.
struct ScrobbleState {
    track: Track,
    started_at: i64,
    submitted: bool,
    last_position: f64,
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
pub(crate) fn album_key(album: &str, artist: &str) -> String {
    format!("{}\u{1f}{}", album.to_lowercase(), artist.to_lowercase())
}

/// Cache key for a track's album artwork.
pub(crate) fn track_cover_key(track: &Track) -> String {
    let artist = if track.album_artist.trim().is_empty() {
        &track.artist
    } else {
        &track.album_artist
    };
    album_key(&track.album, artist)
}

/// Upper bound for cached album textures. At 180x180 RGBA this is roughly
/// 25 MiB of CPU-side image data, plus the GPU copies.
const ALBUM_COVER_CACHE_CAPACITY: usize = 192;

/// Maximum covers decoded in parallel. More requests than this wait in the
/// channel, which keeps fast scrolling from queueing the whole library.
const ALBUM_COVER_WORKERS: usize = 3;

/// Keep the queue short so covers for the current viewport are not stuck
/// behind hundreds of tiles the user already scrolled past.
const ALBUM_COVER_PENDING_LIMIT: usize = 48;

/// One cover still being loaded in the background.
struct CoverJob {
    key: String,
    path: PathBuf,
    id: u64,
}

/// Small LRU cache for album artwork. Negative results are cached too, so a
/// coverless album is not re-read on every frame. Decoding runs on worker
/// threads; the UI only uploads finished textures.
pub(crate) struct CoverCache {
    entries: HashMap<String, Option<egui::TextureHandle>>,
    order: VecDeque<String>,
    capacity: usize,
    pending: HashMap<String, u64>,
    next_request: u64,
    jobs: Option<std::sync::mpsc::Sender<CoverJob>>,
    done: Option<std::sync::mpsc::Receiver<(String, u64, Option<egui::ColorImage>)>>,
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::with_capacity(ALBUM_COVER_CACHE_CAPACITY)
    }
}

impl CoverCache {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
            pending: HashMap::new(),
            next_request: 0,
            jobs: None,
            done: None,
        }
    }

    /// Upload finished covers and drop them from the pending set. Call once at
    /// the start of every frame.
    pub(crate) fn pump(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        if let Some(done) = &self.done {
            while let Ok(item) = done.try_recv() {
                finished.push(item);
            }
        }
        for (key, id, image) in finished {
            // A cover replaced while its old job was running must not come back.
            if self.pending.get(&key) != Some(&id) {
                continue;
            }
            self.pending.remove(&key);
            let texture = image.map(|color| {
                ctx.load_texture(
                    format!("album-cover-{key}"),
                    color,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.insert(key, texture);
        }
    }

    /// True while a cover for `key` is being decoded.
    pub(crate) fn is_pending(&self, key: &str) -> bool {
        self.pending.contains_key(key)
    }

    fn request(&mut self, ctx: &egui::Context, key: &str, path: &Path) {
        if self.pending.contains_key(key) {
            return;
        }
        if self.jobs.is_none() {
            let (job_tx, job_rx) = std::sync::mpsc::channel::<CoverJob>();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
            for _ in 0..ALBUM_COVER_WORKERS {
                let job_rx = std::sync::Arc::clone(&job_rx);
                let done_tx = done_tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || loop {
                    let job = {
                        let guard = job_rx.lock().unwrap_or_else(|error| error.into_inner());
                        guard.recv()
                    };
                    let Ok(job) = job else {
                        break;
                    };
                    let image = decode_cover(&job.path);
                    if done_tx.send((job.key, job.id, image)).is_err() {
                        break;
                    }
                    // Wake the UI thread so the finished texture is uploaded
                    // even when the app is otherwise idle.
                    ctx.request_repaint();
                });
            }
            self.jobs = Some(job_tx);
            self.done = Some(done_rx);
        }
        if self.pending.len() >= ALBUM_COVER_PENDING_LIMIT {
            return;
        }
        self.next_request = self.next_request.wrapping_add(1);
        let id = self.next_request;
        let sent = self.jobs.as_ref().is_some_and(|jobs| {
            jobs.send(CoverJob {
                key: key.to_string(),
                path: path.to_path_buf(),
                id,
            })
            .is_ok()
        });
        if sent {
            self.pending.insert(key.to_string(), id);
        }
    }

    /// `Some(cached)` when the key is present, including a cached `None`.
    pub(crate) fn get(&mut self, key: &str) -> Option<Option<egui::TextureHandle>> {
        let value = self.entries.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    pub(crate) fn insert(&mut self, key: String, value: Option<egui::TextureHandle>) {
        if self.entries.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
        } else {
            self.touch(&key);
        }
        while self.entries.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    pub(crate) fn remove(&mut self, key: &str) {
        self.entries.remove(key);
        self.order.retain(|item| item != key);
        self.pending.remove(key);
    }

    /// Drop cached covers that are no longer part of the album list.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|key, _| keep(key));
        self.order.retain(|key| keep(key));
        self.pending.retain(|key, _| keep(key));
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn touch(&mut self, key: &str) {
        if let Some(index) = self.order.iter().position(|item| item == key) {
            if let Some(item) = self.order.remove(index) {
                self.order.push_back(item);
            }
        }
    }
}

/// Request a cover texture for one album. Returns immediately: the decode runs
/// on a worker thread and the next [`CoverCache::pump`] uploads the texture.
/// Shared by the album grid and the album-art track layout.
pub(crate) fn load_cover_texture(
    ctx: &egui::Context,
    covers: &mut CoverCache,
    key: &str,
    path: &Path,
) -> Option<egui::TextureHandle> {
    if let Some(cached) = covers.get(key) {
        return cached;
    }
    covers.request(ctx, key, path);
    None
}

fn decode_cover(path: &Path) -> Option<egui::ColorImage> {
    let bytes = musicwasp_core::library::cover_art(path)?;
    let image = crate::images::from_bytes(&bytes).ok()?;
    let image = image.resize(180, 180, image::imageops::FilterType::Triangle);
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        &rgba.into_raw(),
    ))
}

fn image_mime(data: &[u8]) -> Option<&'static str> {
    match image::guess_format(data).ok()? {
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::Bmp => Some("image/bmp"),
        image::ImageFormat::Tiff => Some("image/tiff"),
        image::ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

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

fn auto_number_tracks(edits: &mut [TagEdit]) {
    let mut counters: BTreeMap<i32, i32> = BTreeMap::new();
    for edit in edits {
        let disc = edit.disc_no.unwrap_or(1);
        let counter = counters.entry(disc).or_insert(0);
        *counter += 1;
        edit.track_no = Some(*counter);
    }
}

fn normalize_case_value(value: &str, style: BatchCase) -> String {
    match style {
        BatchCase::Lower => value.to_lowercase(),
        BatchCase::Upper => value.to_uppercase(),
        BatchCase::Title => value
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                let Some(first) = chars.next() else {
                    return String::new();
                };
                let mut out = first.to_uppercase().collect::<String>();
                out.push_str(&chars.as_str().to_lowercase());
                out
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
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
        replay_gain: None,
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
    pub output_devices: Vec<String>,

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
    /// Optional play-history filters, toggled from the main menu.
    pub filter_never_played: bool,
    pub filter_not_played_90: bool,
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
    /// Track id for which the next gapless source has been queued.
    gapless_for: Option<i64>,
    scrobble: Option<ScrobbleState>,
    scrobble_tx: Sender<String>,
    scrobble_rx: Receiver<String>,

    // Misc
    pub scan_path: String,
    scan: Option<ScanState>,
    pub message: String,
    pub art: Option<(String, egui::TextureHandle)>,
    pub art_key: Option<String>,
    /// Toolbar brand mark.
    pub logo: Option<egui::TextureHandle>,
    /// Full-window now-playing view instead of browser + track list.
    pub plain_view: bool,
    pub playing_id: Option<i64>,
    /// Last track looked up for the playing file. Cleared when the library changes.
    current_track_cache: Option<(PathBuf, Option<Track>)>,
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
    pub playlist_folders: Vec<PlaylistFolder>,
    pub smart_playlists: Vec<SmartPlaylist>,
    pub smart_dialog: Option<SmartDraft>,
    pub playlist_dialog: Option<String>,
    /// When true the new playlist does not copy the current view/selection.
    pub playlist_dialog_empty: bool,
    /// Folder that will receive the new playlist.
    pub playlist_dialog_folder: Option<i64>,
    pub folder_dialog: Option<FolderDraft>,
    pub m3u_dialog: Option<M3uDraft>,
    pub saved_searches: Vec<SavedSearch>,
    pub saved_search_dialog: Option<SavedSearchDraft>,
    pub picker: Option<FolderPicker>,
    pub autotag: Option<AutoTagState>,
    pub show_hygiene: bool,
    pub hygiene: Option<HygieneState>,
    pub show_organize: bool,
    pub organize: Option<OrganizeState>,
    pub restore_dialog: Option<RestoreDraft>,
    /// Journal of the last organizer batch, so moves can be undone.
    pub move_undo: Option<Vec<musicwasp_core::organize::MoveRecord>>,
    move_undo_path: PathBuf,
    /// Compact always-on-top window bridge.
    mini: Option<crate::mini_player::MiniBridge>,
    /// Optional Linux status-notifier tray.
    #[cfg(target_os = "linux")]
    tray: Option<crate::tray::TrayController>,
    pub batch_case: BatchCase,
    pub batch_field: BatchField,
    pub batch_copy: BatchField,
    /// Album grid mode and its cached covers.
    pub album_view: bool,
    pub album_groups: Vec<AlbumGroup>,
    pub(crate) album_covers: CoverCache,
    albums_dirty: bool,
    /// MilkDrop visualizer state.
    pub wgpu_state: Option<eframe::egui_wgpu::RenderState>,
    pub visualizer: Option<crate::visualizer::Visualizer>,
    pub viz_error: Option<String>,
    pub viz_fullscreen: bool,
    pub viz_pcm: Vec<f32>,
    pub viz_channels: u16,
    /// Lyrics for the current local track and their sidecar path key.
    pub lyrics: Option<Lyrics>,
    lyrics_track: Option<PathBuf>,
    /// Line highlighted last frame, used to auto-scroll only on changes.
    pub lyrics_active: Option<usize>,
    pub lyrics_autoscroll: bool,
    /// Internet radio and podcast playback.
    pub stream: StreamState,
    pub podcast: PodcastState,
    pub radio: RadioState,
    /// Track-list column settings window.
    pub show_columns: bool,
    /// Ask the table to discard its cached widths on the next frame.
    pub columns_reset: bool,
    cover_rx: Option<Receiver<CoverMsg>>,
    pub cover_picker: Option<CoverPickerState>,
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
    /// Snapshot of the tags overwritten by the last write, for Undo.
    pub tag_undo: Option<Vec<TagSnapshot>>,
    tag_undo_path: PathBuf,
    /// Last session written to disk, used to avoid rewriting settings each frame.
    last_session: Session,
    last_session_save: std::time::Instant,
    /// Last known window geometry; restored at startup and written on exit.
    window_geometry: Option<WindowGeometry>,
    /// Screenshot runs must be deterministic, so they skip session restore/save.
    session_enabled: bool,
    pub show_settings: bool,
    pub show_skin_editor: bool,
    pub skin_editor: Option<SkinEditorState>,
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
        crate::theme::load_user_skins();
        let settings_path = Settings::path();
        // Test runs keep the developer's real credential store out of reach.
        let mut settings = if debug_env::is_set("MUSICWASP_TEST") {
            Settings::load(&settings_path)
        } else {
            Settings::load_persisted(&settings_path)
        };
        let tag_undo_path = settings_path.with_file_name("tag-undo.json");
        let tag_undo = musicwasp_core::tags::load_undo(&tag_undo_path)
            .ok()
            .flatten();
        let move_undo_path = settings_path.with_file_name("moves-undo.json");
        let move_undo = musicwasp_core::organize::load_undo(&move_undo_path)
            .ok()
            .flatten();
        settings.equalizer = settings.equalizer.clamped();
        crate::i18n::set_language(settings.language);
        crate::icons::install(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx, settings.skin);
        crate::theme::apply_text_scale(&cc.egui_ctx, settings.large_fonts);
        let wgpu_state = cc.wgpu_render_state.clone();

        // Headless UI tests use a temporary data dir and no audio/watcher/MPRIS.
        let test_mode = debug_env::is_set("MUSICWASP_TEST");
        let lib = Library::open().expect("cannot open library database");
        let (player, audio_error) = if test_mode {
            (None, None)
        } else {
            match Player::new_with_device(settings.output_device.clone()) {
                Ok(player) => (Some(player), None),
                Err(e) if settings.output_device.is_some() => match Player::new_with_device(None) {
                    Ok(player) => (
                        Some(player),
                        Some(format!(
                            "selected output unavailable ({e}); using the default"
                        )),
                    ),
                    Err(fallback) => (None, Some(fallback.to_string())),
                },
                Err(e) => (None, Some(e.to_string())),
            }
        };
        let output_devices = if test_mode {
            Vec::new()
        } else {
            musicwasp_core::player::output_devices()
        };
        let (scrobble_tx, scrobble_rx) = mpsc::channel();

        let scan_path = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/music".to_string());

        let eq_applied = settings.equalizer;

        let shot_path = debug_env::var("MUSICWASP_SCREENSHOT");
        let session_enabled = shot_path.is_none() && !test_mode;
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
            output_devices,
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
            filter_never_played: false,
            filter_not_played_90: false,
            selected: session.selected.iter().copied().collect(),
            anchor,
            queue: session.queue.clone(),
            queue_pos: session.queue_pos,
            repeat: RepeatMode::Off,
            shuffle: false,
            seek_drag: None,
            volume,
            volume_sent: -1.0,
            gapless_for: None,
            scrobble: None,
            scrobble_tx,
            scrobble_rx,
            scan_path,
            scan: None,
            message: String::new(),
            art: None,
            art_key: None,
            logo: None,
            plain_view: debug_env::is_set("MUSICWASP_PLAIN"),
            playing_id: None,
            current_track_cache: None,
            editing: None,
            rename: None,
            rename_field: TrackField::default(),
            focus: Focus::default(),
            os_drop_zones: Vec::new(),
            os_drop_hover: None,
            view: session.view.into(),
            playlists: Vec::new(),
            playlist_folders: Vec::new(),
            smart_playlists: Vec::new(),
            smart_dialog: None,
            playlist_dialog: None,
            playlist_dialog_empty: false,
            playlist_dialog_folder: None,
            folder_dialog: None,
            m3u_dialog: None,
            saved_searches: Vec::new(),
            saved_search_dialog: None,
            picker: None,
            autotag: None,
            show_hygiene: false,
            hygiene: None,
            show_organize: false,
            organize: None,
            restore_dialog: None,
            batch_case: BatchCase::default(),
            batch_field: BatchField::default(),
            batch_copy: BatchField::default(),
            album_view: false,
            album_groups: Vec::new(),
            album_covers: CoverCache::default(),
            albums_dirty: true,
            wgpu_state,
            visualizer: None,
            viz_error: None,
            viz_fullscreen: debug_env::is_set("MUSICWASP_VIS_FULLSCREEN"),
            viz_pcm: Vec::new(),
            viz_channels: 2,
            lyrics: None,
            lyrics_track: None,
            lyrics_active: None,
            lyrics_autoscroll: true,
            stream: StreamState::default(),
            podcast: PodcastState::default(),
            radio: RadioState::default(),
            show_columns: false,
            columns_reset: false,
            cover_rx: None,
            cover_picker: None,
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
            tag_undo,
            tag_undo_path,
            move_undo,
            move_undo_path,
            mini: None,
            #[cfg(target_os = "linux")]
            tray: None,
            last_session: Session::default(),
            last_session_save: std::time::Instant::now(),
            window_geometry,
            session_enabled,
            show_settings: false,
            show_skin_editor: false,
            skin_editor: None,
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
        app.refresh_podcast_feeds();
        app.refresh_radio_stations();
        debug_hooks::apply_initial(&mut app);
        if app.session_enabled {
            app.restore_session(&session);
        }
        app.last_session = app.capture_session();
        app.refresh_brand_icon(&cc.egui_ctx);
        if let Some(player) = &app.player {
            player.set_equalizer(app.settings.equalizer);
            player.set_replaygain(app.settings.replaygain, app.settings.replaygain_album);
            player.set_crossfade(app.settings.crossfade_secs);
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
            never_played: self.filter_never_played,
            not_played_days: if self.filter_not_played_90 {
                Some(90)
            } else {
                None
            },
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
        self.playlist_folders = self.lib.playlist_folders().unwrap_or_default();
        self.smart_playlists = self.lib.smart_playlists().unwrap_or_default();
        self.saved_searches = self.lib.saved_searches().unwrap_or_default();
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
        if matches!(self.view, View::Podcasts | View::Radio) {
            self.dirty_tracks = false;
            return;
        }
        let result = match self.view {
            View::Library => self.lib.query(&self.filter(), self.sort, self.sort_asc),
            View::Playlist(id) => self.lib.playlist_tracks(id),
            View::Smart(id) => self.lib.smart_playlist_tracks(id),
            View::Podcasts | View::Radio => Ok(Vec::new()),
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
            View::Library => tr("library.title").to_string(),
            View::Playlist(id) => self
                .playlists
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("Playlist · {}", p.name))
                .unwrap_or_else(|| tr("library.title").to_string()),
            View::Smart(id) => self
                .smart_playlists
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("Smart · {}", p.name))
                .unwrap_or_else(|| tr("library.title").to_string()),
            View::Podcasts => tr("podcasts.title").to_string(),
            View::Radio => tr("radio.title").to_string(),
        }
    }

    /// Open the new-playlist dialog. Empty playlists skip the current view.
    /// `folder` places the result inside a playlist folder.
    pub fn open_playlist_dialog(&mut self, empty: bool, folder: Option<i64>) {
        let n = self.playlists.len() + 1;
        self.playlist_dialog = Some(format!("Playlist {n}"));
        self.playlist_dialog_empty = empty;
        self.playlist_dialog_folder = folder;
    }

    /// Save the currently displayed tracks as a new playlist, or create an
    /// empty one when the dialog requested it.
    pub fn commit_playlist_dialog(&mut self) {
        let Some(name) = self.playlist_dialog.clone() else {
            return;
        };
        let selected = self.selected_in_view();
        let ids: Vec<i64> = if self.playlist_dialog_empty {
            Vec::new()
        } else if selected.is_empty() {
            self.tracks.iter().map(|t| t.id).collect()
        } else {
            selected
        };
        match self.lib.create_playlist(&name, &ids) {
            Ok(id) => {
                let folder = self.playlist_dialog_folder;
                let moved = match folder {
                    Some(folder) => self
                        .lib
                        .move_playlist_to_folder(id, Some(folder))
                        .map_err(|error| format!("created, but could not move to folder: {error}")),
                    None => Ok(()),
                };
                self.message = match moved {
                    Ok(()) if self.playlist_dialog_empty => {
                        format!("empty playlist \"{}\" created", name.trim())
                    }
                    Ok(()) => {
                        format!("playlist \"{}\" saved ({} tracks)", name.trim(), ids.len())
                    }
                    Err(error) => error,
                };
                self.playlist_dialog = None;
                self.playlist_dialog_empty = false;
                self.playlist_dialog_folder = None;
                self.view = View::Playlist(id);
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("playlist failed: {e}"),
        }
    }

    // ---- playlist folders ----

    pub fn open_new_folder(&mut self, parent_id: Option<i64>) {
        self.folder_dialog = Some(FolderDraft {
            name: String::new(),
            parent_id,
            status: None,
        });
    }

    pub fn commit_folder_dialog(&mut self) {
        let Some(mut draft) = self.folder_dialog.clone() else {
            return;
        };
        let name = draft.name.trim().to_string();
        if name.is_empty() {
            draft.status = Some("give the folder a name".to_string());
            self.folder_dialog = Some(draft);
            return;
        }
        match self.lib.create_playlist_folder(&name, draft.parent_id) {
            Ok(_) => {
                self.message = format!("folder \"{name}\" created");
                self.folder_dialog = None;
                self.mark_browser_dirty();
            }
            Err(e) => {
                draft.status = Some(format!("create failed: {e}"));
                self.folder_dialog = Some(draft);
            }
        }
    }

    pub fn delete_playlist_folder(&mut self, folder_id: i64) {
        let name = self
            .playlist_folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.name.clone())
            .unwrap_or_else(|| "folder".to_string());
        match self.lib.delete_playlist_folder(folder_id) {
            Ok(()) => {
                self.message = format!("folder \"{name}\" deleted; its playlists moved up");
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("delete folder failed: {e}"),
        }
    }

    pub fn move_playlist_to_folder(&mut self, playlist_id: i64, folder_id: Option<i64>) {
        let result = self.lib.move_playlist_to_folder(playlist_id, folder_id);
        self.report_playlist_move(result, "playlist", folder_id);
    }

    pub fn move_smart_to_folder(&mut self, playlist_id: i64, folder_id: Option<i64>) {
        let result = self
            .lib
            .move_smart_playlist_to_folder(playlist_id, folder_id);
        self.report_playlist_move(result, "smart playlist", folder_id);
    }

    fn report_playlist_move(
        &mut self,
        result: anyhow::Result<()>,
        what: &str,
        folder_id: Option<i64>,
    ) {
        match result {
            Ok(()) => {
                self.message = if folder_id.is_some() {
                    format!("{what} moved to folder")
                } else {
                    format!("{what} moved to the top level")
                };
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("move failed: {e}"),
        }
    }

    /// Full `Parent / Child` path for menus. Unknown ids render as `Folder`.
    pub fn playlist_folder_path(&self, folder_id: i64) -> String {
        let mut names = Vec::new();
        let mut current = Some(folder_id);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                break;
            }
            let Some(folder) = self.playlist_folders.iter().find(|folder| folder.id == id) else {
                break;
            };
            names.push(folder.name.clone());
            current = folder.parent_id;
        }
        names.reverse();
        names.join(" / ")
    }

    pub fn start_rename_folder(&mut self, folder_id: i64) {
        let Some(folder) = self
            .playlist_folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .cloned()
        else {
            self.message = "that folder no longer exists".to_string();
            return;
        };
        self.begin_rename(RenameTarget::Folder(folder_id), folder.name);
    }

    pub fn start_rename_playlist(&mut self, playlist_id: i64) {
        let Some(playlist) = self
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .cloned()
        else {
            self.message = "that playlist no longer exists".to_string();
            return;
        };
        self.begin_rename(RenameTarget::Playlist(playlist_id), playlist.name);
    }

    pub fn start_rename_smart(&mut self, playlist_id: i64) {
        let Some(playlist) = self
            .smart_playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .cloned()
        else {
            self.message = "that smart playlist no longer exists".to_string();
            return;
        };
        self.begin_rename(RenameTarget::Smart(playlist_id), playlist.name);
    }

    fn begin_rename(&mut self, target: RenameTarget, text: String) {
        self.rename = Some(RenameState {
            target,
            text,
            error: None,
            request_focus: true,
        });
    }

    // ---- M3U import / export ----

    pub fn open_m3u_import(&mut self) {
        self.m3u_dialog = Some(M3uDraft {
            mode: M3uMode::Import,
            path: String::new(),
            name: String::new(),
            folder_id: None,
            status: None,
        });
    }

    pub fn open_m3u_export(&mut self, playlist_id: i64) {
        let Some(name) = self
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
        else {
            self.message = "that playlist no longer exists".to_string();
            return;
        };
        self.open_m3u_export_dialog(M3uMode::Export(playlist_id), &name);
    }

    pub fn open_smart_m3u_export(&mut self, playlist_id: i64) {
        let Some(name) = self
            .smart_playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
        else {
            self.message = "that smart playlist no longer exists".to_string();
            return;
        };
        self.open_m3u_export_dialog(M3uMode::ExportSmart(playlist_id), &name);
    }

    fn open_m3u_export_dialog(&mut self, mode: M3uMode, name: &str) {
        let dir = dirs::audio_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let file = format!("{}.m3u8", sanitize_file_name(name));
        self.m3u_dialog = Some(M3uDraft {
            mode,
            path: dir.join(file).to_string_lossy().to_string(),
            name: name.to_string(),
            folder_id: None,
            status: None,
        });
    }

    /// Import one M3U file and return the summary message. Used by the dialog
    /// and by OS file drops.
    pub fn import_m3u_file(
        &mut self,
        path: &Path,
        folder_id: Option<i64>,
    ) -> Result<String, String> {
        let report = self
            .lib
            .import_m3u(path, None, folder_id)
            .map_err(|e| e.to_string())?;
        let mut summary = format!("imported \"{}\" ({} track(s))", report.name, report.added);
        if report.missing > 0 {
            summary.push_str(&format!(", {} missing", report.missing));
        }
        if report.unsupported > 0 {
            summary.push_str(&format!(", {} skipped", report.unsupported));
        }
        self.view = View::Playlist(report.playlist_id);
        self.mark_browser_dirty();
        self.mark_tracks_dirty();
        Ok(summary)
    }

    pub fn commit_m3u_dialog(&mut self) {
        let Some(mut draft) = self.m3u_dialog.clone() else {
            return;
        };
        let raw_path = draft.path.trim().to_string();
        if raw_path.is_empty() {
            draft.status = Some("choose a playlist file".to_string());
            self.m3u_dialog = Some(draft);
            return;
        }
        let mut path = PathBuf::from(&raw_path);
        match draft.mode {
            M3uMode::Import => {
                let name = draft.name.trim();
                let result = self.lib.import_m3u(
                    &path,
                    if name.is_empty() { None } else { Some(name) },
                    draft.folder_id,
                );
                match result {
                    Ok(report) => {
                        let mut summary =
                            format!("imported \"{}\" ({} track(s))", report.name, report.added);
                        if report.missing > 0 {
                            summary.push_str(&format!(", {} missing", report.missing));
                        }
                        if report.unsupported > 0 {
                            summary.push_str(&format!(", {} skipped", report.unsupported));
                        }
                        self.message = summary;
                        self.m3u_dialog = None;
                        self.view = View::Playlist(report.playlist_id);
                        self.mark_browser_dirty();
                        self.mark_tracks_dirty();
                    }
                    Err(e) => {
                        draft.status = Some(format!("import failed: {e}"));
                        self.m3u_dialog = Some(draft);
                    }
                }
            }
            M3uMode::Export(playlist_id) | M3uMode::ExportSmart(playlist_id) => {
                if path.extension().is_none() {
                    path.set_extension("m3u8");
                }
                let result = if matches!(draft.mode, M3uMode::ExportSmart(_)) {
                    self.lib.export_smart_m3u(playlist_id, &path)
                } else {
                    self.lib.export_m3u(playlist_id, &path)
                };
                match result {
                    Ok(count) => {
                        self.message = format!("exported {count} track(s) to {}", path.display());
                        self.m3u_dialog = None;
                    }
                    Err(e) => {
                        draft.status = Some(format!("export failed: {e}"));
                        self.m3u_dialog = Some(draft);
                    }
                }
            }
        }
    }

    // ---- saved searches ----

    /// Apply a search string from a saved search or the UI. Column-browser
    /// selections are cleared so the saved query gives a reproducible result.
    pub fn apply_search(&mut self, query: &str) {
        self.search = query.to_string();
        self.sel_genre = None;
        self.sel_artist = None;
        self.sel_album = None;
        self.mark_browser_dirty();
        self.mark_tracks_dirty();
    }

    pub fn open_save_search_dialog(&mut self) {
        if self.search.trim().is_empty() {
            self.message = "type a search first".to_string();
            return;
        }
        let count = self.saved_searches.len() + 1;
        self.saved_search_dialog = Some(SavedSearchDraft {
            id: None,
            name: format!("Search {count}"),
            query: self.search.clone(),
            status: None,
        });
    }

    pub fn edit_saved_search(&mut self, id: i64) {
        let Some(saved) = self
            .saved_searches
            .iter()
            .find(|saved| saved.id == id)
            .cloned()
        else {
            self.message = "that saved search no longer exists".to_string();
            return;
        };
        self.saved_search_dialog = Some(SavedSearchDraft {
            id: Some(id),
            name: saved.name,
            query: saved.query,
            status: None,
        });
    }

    pub fn commit_saved_search_dialog(&mut self) {
        let Some(mut draft) = self.saved_search_dialog.clone() else {
            return;
        };
        let name = draft.name.trim().to_string();
        let query = draft.query.trim().to_string();
        if name.is_empty() {
            draft.status = Some("give the search a name".to_string());
            self.saved_search_dialog = Some(draft);
            return;
        }
        if query.is_empty() {
            draft.status = Some("the query cannot be empty".to_string());
            self.saved_search_dialog = Some(draft);
            return;
        }
        let result = match draft.id {
            Some(id) => self.lib.update_saved_search(id, &name, &query),
            None => self.lib.create_saved_search(&name, &query).map(|_| ()),
        };
        match result {
            Ok(()) => {
                self.message = if draft.id.is_some() {
                    format!("saved search \"{name}\" updated")
                } else {
                    format!("saved search \"{name}\" saved")
                };
                self.saved_search_dialog = None;
                self.saved_searches = self.lib.saved_searches().unwrap_or_default();
                if draft.id.is_some() {
                    self.apply_search(&query);
                }
            }
            Err(e) => {
                draft.status = Some(format!("save failed: {e}"));
                self.saved_search_dialog = Some(draft);
            }
        }
    }

    pub fn delete_saved_search(&mut self, id: i64) {
        let name = self
            .saved_searches
            .iter()
            .find(|saved| saved.id == id)
            .map(|saved| saved.name.clone())
            .unwrap_or_else(|| "search".to_string());
        match self.lib.delete_saved_search(id) {
            Ok(()) => {
                self.saved_searches = self.lib.saved_searches().unwrap_or_default();
                self.message = format!("saved search \"{name}\" deleted");
            }
            Err(e) => self.message = format!("delete saved search failed: {e}"),
        }
    }

    pub fn delete_current_playlist(&mut self) {
        let result = match self.view {
            View::Playlist(id) => self.lib.delete_playlist(id),
            View::Smart(id) => self.lib.delete_smart_playlist(id),
            View::Library | View::Podcasts | View::Radio => return,
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

    pub fn delete_playlist_by_id(&mut self, playlist_id: i64) {
        match self.lib.delete_playlist(playlist_id) {
            Ok(()) => {
                if matches!(self.view, View::Playlist(id) if id == playlist_id) {
                    self.view = View::Library;
                    self.mark_tracks_dirty();
                }
                self.message = "playlist deleted".to_string();
                self.mark_browser_dirty();
            }
            Err(e) => self.message = format!("delete failed: {e}"),
        }
    }

    pub fn delete_smart_by_id(&mut self, playlist_id: i64) {
        match self.lib.delete_smart_playlist(playlist_id) {
            Ok(()) => {
                if matches!(self.view, View::Smart(id) if id == playlist_id) {
                    self.view = View::Library;
                    self.mark_tracks_dirty();
                }
                self.message = "smart playlist deleted".to_string();
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
        self.open_smart_dialog_in(None);
    }

    /// Create a smart playlist inside `folder`, or at the root when `None`.
    pub fn open_smart_dialog_in(&mut self, folder: Option<i64>) {
        let mut definition = Definition::default();
        definition.rules.push(Rule::new(Field::Genre, Op::Contains));
        self.smart_dialog = Some(SmartDraft {
            name: format!("Smart playlist {}", self.smart_playlists.len() + 1),
            definition,
            status: None,
            folder,
            editing: None,
        });
    }

    /// Open the rule editor for an existing smart playlist.
    pub fn edit_smart_dialog(&mut self, id: i64) {
        let Some(smart) = self
            .smart_playlists
            .iter()
            .find(|smart| smart.id == id)
            .cloned()
        else {
            self.message = "that smart playlist no longer exists".to_string();
            return;
        };
        self.smart_dialog = Some(SmartDraft {
            name: smart.name,
            definition: smart.definition,
            status: None,
            folder: smart.folder_id,
            editing: Some(id),
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
        if let Some(id) = draft.editing {
            let result = self
                .lib
                .update_smart_playlist(id, &draft.definition)
                .and_then(|()| self.lib.rename_smart_playlist(id, &name));
            match result {
                Ok(()) => {
                    self.message = format!("smart playlist \"{name}\" updated");
                    self.smart_dialog = None;
                    self.view = View::Smart(id);
                    self.mark_browser_dirty();
                }
                Err(e) => draft.status = Some(format!("update failed: {e}")),
            }
            return;
        }
        match self
            .lib
            .create_smart_playlist(&name, &draft.definition, draft.folder)
        {
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
        self.albums_dirty = true;
        self.current_track_cache = None;
    }

    pub fn mark_tracks_dirty(&mut self) {
        self.dirty_tracks = true;
        self.current_track_cache = None;
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
            reconcile: None,
            rename_selected: BTreeSet::new(),
            pending_renames: None,
            status: None,
        });
    }

    /// Read-only rename detection: match vanished rows against files on disk.
    /// Nothing is written until the preview is applied.
    pub fn reconcile_hygiene(&mut self) {
        if self.hygiene.is_none() {
            self.scan_hygiene();
        }
        let roots: Vec<PathBuf> = self
            .settings
            .watch_folders
            .iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
        let status = if roots.is_empty() {
            "Scan a folder first: renames are detected inside watched folders.".to_string()
        } else {
            match self.lib.reconcile(&roots) {
                Ok(plan) => {
                    let text = format!(
                        "{} vanished row(s), {} possible rename(s), {} file(s) scanned",
                        plan.missing.len(),
                        plan.candidates.len(),
                        plan.scanned_files
                    );
                    if let Some(state) = &mut self.hygiene {
                        state.rename_selected =
                            plan.candidates.iter().map(|c| c.track.id).collect();
                        state.reconcile = Some(plan);
                        state.pending_renames = None;
                    }
                    text
                }
                Err(e) => format!("could not scan for renames: {e}"),
            }
        };
        self.hygiene_message(status);
    }

    /// Apply the accepted relocation preview. Audio files are never modified.
    pub fn apply_hygiene_renames(&mut self, ids: &[i64]) {
        let Some(state) = &self.hygiene else {
            return;
        };
        let Some(plan) = &state.reconcile else {
            return;
        };
        let wanted: BTreeSet<i64> = ids.iter().copied().collect();
        let candidates: Vec<_> = plan
            .candidates
            .iter()
            .filter(|candidate| wanted.contains(&candidate.track.id))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return;
        }
        match self.lib.apply_reconcile(&candidates) {
            Ok(outcome) => {
                let removed: BTreeSet<i64> = outcome.removed.into_iter().collect();
                self.forget_tracks(&removed);
                self.finish_hygiene(format!(
                    "relocated {} track(s) to their new paths",
                    outcome.moved
                ));
            }
            Err(e) => self.hygiene_message(format!("could not apply renames: {e}")),
        }
    }

    /// Remove rows from the library only. Audio files are never deleted.
    pub fn remove_from_library(&mut self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        let removed: BTreeSet<i64> = ids.iter().copied().collect();
        match self.lib.remove_tracks(ids) {
            Ok(count) => {
                self.forget_tracks(&removed);
                self.finish_hygiene(format!(
                    "removed {count} track(s) from the library; audio files were kept"
                ));
            }
            Err(e) => {
                self.hygiene_message(format!("could not remove tracks from the library: {e}"))
            }
        }
    }

    /// Drop removed rows from the queue and the selection, stopping playback
    /// when the current track was one of them.
    fn forget_tracks(&mut self, removed: &BTreeSet<i64>) {
        if removed.is_empty() {
            return;
        }
        let (queue_pos, current_removed) =
            remove_tracks_from_queue(&mut self.queue, self.queue_pos, removed);
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
    }

    /// Show a maintenance result in the toolbar and in the open window.
    fn hygiene_message(&mut self, message: String) {
        self.message = message;
        if let Some(state) = &mut self.hygiene {
            state.status = Some(self.message.clone());
        }
    }

    /// Publish a maintenance result, rescan the hygiene lists, refresh views.
    fn finish_hygiene(&mut self, message: String) {
        self.message = message;
        self.scan_hygiene();
        if let Some(state) = &mut self.hygiene {
            state.status = Some(self.message.clone());
        }
        self.mark_browser_dirty();
    }

    // ---- file organizer ----

    /// Open the organizer for the current selection (or the whole library).
    pub fn open_organize(&mut self) {
        self.show_organize = true;
        if self.organize.is_none() {
            let recorded_root = self.settings.organize_root.trim();
            let root = if recorded_root.is_empty() {
                self.settings
                    .watch_folders
                    .first()
                    .cloned()
                    .unwrap_or_else(|| self.scan_path.clone())
            } else {
                recorded_root.to_string()
            };
            let pattern = self.settings.organize_pattern.trim();
            self.organize = Some(OrganizeState {
                pattern: if pattern.is_empty() {
                    musicwasp_core::organize::DEFAULT_PATTERN.to_string()
                } else {
                    pattern.to_string()
                },
                root,
                selected_only: !self.selected_in_view().is_empty(),
                plan: None,
                pending: false,
                status: None,
            });
        }
        self.organize_preview();
    }

    /// Rebuild the dry-run plan. Never moves or renames anything.
    pub fn organize_preview(&mut self) {
        let Some(state) = self.organize.as_ref() else {
            return;
        };
        let root = PathBuf::from(state.root.trim());
        let pattern = state.pattern.clone();
        let selected_only = state.selected_only;
        if !root.is_dir() {
            if let Some(state) = &mut self.organize {
                state.plan = None;
                state.pending = false;
                state.status = Some(format!("not a folder: {}", root.display()));
            }
            return;
        }

        let tracks = if selected_only {
            let ids = self.selected_in_view();
            if ids.is_empty() {
                if let Some(state) = &mut self.organize {
                    state.plan = None;
                    state.pending = false;
                    state.status = Some("select a track first".to_string());
                }
                return;
            }
            self.lib.tracks_by_ids(&ids).unwrap_or_default()
        } else {
            self.lib
                .query(&Filter::default(), SortKey::Artist, true)
                .unwrap_or_default()
        };

        let result = musicwasp_core::organize::plan(&tracks, &root, &pattern);
        if let Some(state) = &mut self.organize {
            state.pending = false;
            match result {
                Ok(plan) => {
                    state.status = Some(format!(
                        "{} ready to move · {} skipped · {} track(s)",
                        plan.ready,
                        plan.skipped,
                        plan.rows.len()
                    ));
                    state.plan = Some(plan);
                }
                Err(error) => {
                    state.plan = None;
                    state.status = Some(format!("preview failed: {error:#}"));
                }
            }
        }
        self.settings.organize_pattern = pattern;
        self.settings.organize_root = root.to_string_lossy().to_string();
        self.persist_settings();
    }

    /// Move the files of the accepted preview rows and keep an undo journal.
    pub fn organize_apply(&mut self) {
        let Some(plan) = self.organize.as_ref().and_then(|state| state.plan.clone()) else {
            return;
        };
        if !plan.has_moves() {
            return;
        }
        match self.lib.organize_apply(&plan.rows) {
            Ok(report) => {
                if !report.records.is_empty() {
                    let _ =
                        musicwasp_core::organize::save_undo(&self.move_undo_path, &report.records);
                    self.move_undo = Some(report.records);
                }
                self.mark_browser_dirty();
                self.mark_tracks_dirty();
                self.organize_preview();
                let mut text = format!("moved {} file(s)", report.moved);
                if !report.failed.is_empty() {
                    text.push_str(&format!(", {} failed", report.failed.len()));
                    if let Some(first) = report.failed.first() {
                        text.push_str(&format!(": {first}"));
                    }
                }
                self.set_organize_status(text);
            }
            Err(error) => self.set_organize_status(format!("move failed: {error:#}")),
        }
    }

    /// Restore the last organizer batch, files and library rows together.
    pub fn undo_file_moves(&mut self) {
        let Some(records) = self.move_undo.clone() else {
            self.message = "no file moves to undo".to_string();
            return;
        };
        match self.lib.organize_undo(&records) {
            Ok(report) => {
                if report.remaining.is_empty() {
                    musicwasp_core::organize::clear_undo(&self.move_undo_path);
                    self.move_undo = None;
                } else {
                    let _ = musicwasp_core::organize::save_undo(
                        &self.move_undo_path,
                        &report.remaining,
                    );
                    self.move_undo = Some(report.remaining);
                }
                self.mark_browser_dirty();
                self.mark_tracks_dirty();
                if self.show_organize {
                    self.organize_preview();
                }
                let mut text = format!("restored {} file(s)", report.restored);
                if !report.failed.is_empty() {
                    text.push_str(&format!(", {} failed", report.failed.len()));
                }
                self.set_organize_status(text);
            }
            Err(error) => self.set_organize_status(format!("undo failed: {error:#}")),
        }
    }

    fn set_organize_status(&mut self, text: String) {
        self.message = text.clone();
        if let Some(state) = &mut self.organize {
            state.status = Some(text);
        }
    }

    // ---- album grid and cover tools ----

    fn refresh_album_groups(&mut self) {
        match self.lib.album_groups() {
            Ok(groups) => {
                // Keep textures for albums that still exist. Rebuilding the
                // grid (for example after a rating change) should not throw
                // away every decoded cover.
                let keys: std::collections::HashSet<String> = groups
                    .iter()
                    .map(|group| album_key(&group.album, &group.artist))
                    .collect();
                self.album_covers.retain(|key| keys.contains(key));
                self.album_groups = groups;
                self.albums_dirty = false;
            }
            Err(e) => self.message = format!("could not load albums: {e}"),
        }
    }

    /// Cached cover texture for one album tile.
    pub fn album_cover_texture(
        &mut self,
        ctx: &egui::Context,
        group: &AlbumGroup,
    ) -> Option<egui::TextureHandle> {
        let key = album_key(&group.album, &group.artist);
        load_cover_texture(ctx, &mut self.album_covers, &key, Path::new(&group.path))
    }

    pub fn toggle_album_view(&mut self) {
        self.album_view = !self.album_view;
        if self.album_view {
            self.albums_dirty = true;
        }
    }

    /// Library row for the playing file. The UI calls this every frame, so the
    /// last answer is kept until the path changes or the library is marked dirty.
    pub fn current_track(&mut self, path: Option<&Path>) -> Option<Track> {
        let path = path?;
        if let Some((cached, track)) = &self.current_track_cache {
            if cached == path {
                return track.clone();
            }
        }
        let track = self.lib.track_by_path(path).ok().flatten();
        self.current_track_cache = Some((path.to_path_buf(), track.clone()));
        track
    }

    // ---- lyrics ----

    pub fn toggle_lyrics(&mut self) {
        self.settings.lyrics_open = !self.settings.lyrics_open;
        if self.settings.lyrics_open {
            self.lyrics_active = None;
        }
        self.persist_settings();
    }

    /// Load the sidecar file or embedded tag lyrics when the track changes.
    pub fn sync_lyrics(&mut self) {
        let current = self
            .player
            .as_ref()
            .and_then(|player| player.snapshot().current);
        if current == self.lyrics_track {
            return;
        }
        self.lyrics_track = current.clone();
        self.lyrics_active = None;
        self.lyrics = current
            .as_deref()
            .filter(|path| path.is_file())
            .and_then(musicwasp_core::lyrics::load);
    }

    /// Re-read lyrics without waiting for a track change.
    pub fn reload_lyrics(&mut self) {
        self.lyrics_track = None;
        self.lyrics_active = None;
        self.sync_lyrics();
    }

    // ---- track-list columns ----

    /// Remember widths captured from the resizable header.
    pub fn update_column_widths(&mut self, visible: &[TrackColumn], widths: &[f32]) {
        if visible.len() != widths.len() {
            return;
        }
        let mut changed = false;
        for (column, width) in visible.iter().zip(widths) {
            let current = self.settings.columns.width(*column);
            if (current - *width).abs() > 0.5 {
                self.settings.columns.set_width(*column, *width);
                changed = true;
            }
        }
        if changed {
            self.persist_settings();
        }
    }

    /// Leave grid mode and show the tracks of one album.
    pub fn open_album(&mut self, album: &str) {
        self.album_view = false;
        self.sel_genre = None;
        self.sel_artist = None;
        self.sel_album = Some(album.to_string());
        self.search.clear();
        self.mark_browser_dirty();
    }

    pub fn open_cover_picker(&mut self, album: &str, artist: &str, start: PathBuf) {
        self.cover_picker = Some(CoverPickerState::new(
            album.to_string(),
            artist.to_string(),
            start,
        ));
    }

    pub fn apply_cover_path(&mut self, album: &str, artist: &str, path: &Path) {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                self.message = format!("could not read {}: {e}", path.display());
                return;
            }
        };
        let Some(mime) = image_mime(&data) else {
            self.message = format!("not a supported image: {}", path.display());
            return;
        };
        self.cover_picker = None;
        self.apply_album_cover(album, artist, &data, mime);
    }

    /// Write a cover into every track of an album, keeping an undo snapshot.
    fn apply_album_cover(&mut self, album: &str, artist: &str, data: &[u8], mime: &str) {
        let tracks = match self.lib.album_tracks(album, artist) {
            Ok(tracks) => tracks,
            Err(e) => {
                self.message = format!("could not load album tracks: {e}");
                return;
            }
        };
        if tracks.is_empty() {
            self.message = "album has no tracks".to_string();
            return;
        }
        let paths: Vec<PathBuf> = tracks
            .iter()
            .map(|track| PathBuf::from(&track.path))
            .collect();
        if let Err(e) = self.store_tag_undo(&paths) {
            self.message = format!("cover not written: {e}");
            return;
        }

        let mut written = 0usize;
        let mut errors = Vec::new();
        for path in &paths {
            match musicwasp_core::library::embed_cover(path, data, mime) {
                Ok(()) => written += 1,
                Err(e) => errors.push(format!("{}: {e:#}", path.display())),
            }
        }

        self.album_covers.remove(&album_key(album, artist));
        self.art_key = None;
        self.mark_browser_dirty();
        let mut summary = format!("wrote cover to {written} track(s)");
        if !errors.is_empty() {
            summary.push_str(&format!(", {} failed ({})", errors.len(), errors[0]));
        }
        self.message = summary;
    }

    /// Search MusicBrainz/Cover Art Archive for a front cover in the background.
    pub fn fetch_album_cover(&mut self, ctx: &egui::Context, album: &str, artist: &str) {
        if self.cover_rx.is_some() {
            self.message = "a cover lookup is already running".to_string();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let album_name = album.to_string();
        let album_artist = artist.to_string();
        self.cover_rx = Some(rx);
        self.message = format!("looking for a cover for {album}…");

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Option<Cover>> {
                let candidates = autotag::search(&album_artist, &album_name)?;
                for candidate in candidates.iter().take(3) {
                    if let Some(cover) = autotag::front_cover(&candidate.release_id)? {
                        return Ok(Some(cover));
                    }
                }
                Ok(None)
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(CoverMsg {
                album: album_name,
                artist: album_artist,
                result,
            });
            repaint.request_repaint();
        });
    }

    fn poll_cover_fetch(&mut self) {
        let Some(rx) = &self.cover_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(message) => {
                self.cover_rx = None;
                match message.result {
                    Ok(Some(cover)) => self.apply_album_cover(
                        &message.album,
                        &message.artist,
                        &cover.data,
                        &cover.mime,
                    ),
                    Ok(None) => {
                        self.message = format!("no cover found for {}", message.album);
                    }
                    Err(e) => self.message = format!("cover lookup failed: {e}"),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.cover_rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Encode the current cover as `cover.jpg` next to the album's files.
    pub fn save_album_sidecar(&mut self, album: &str, artist: &str) {
        let tracks = match self.lib.album_tracks(album, artist) {
            Ok(tracks) => tracks,
            Err(e) => {
                self.message = format!("could not load album tracks: {e}");
                return;
            }
        };
        let Some(track) = tracks.first() else {
            self.message = "album has no tracks".to_string();
            return;
        };
        let path = PathBuf::from(&track.path);
        let Some(data) = musicwasp_core::library::cover_art(&path) else {
            self.message = "album has no cover to save".to_string();
            return;
        };
        let image = match crate::images::from_bytes(&data) {
            Ok(image) => image,
            Err(e) => {
                self.message = format!("could not decode cover: {e}");
                return;
            }
        };
        let rgb = image.to_rgb8();
        let mut jpeg = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 92);
        if let Err(e) = encoder.encode(
            &rgb,
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        ) {
            self.message = format!("could not encode cover.jpg: {e}");
            return;
        }

        match musicwasp_core::library::save_cover_sidecar(&path, &jpeg) {
            Ok(saved) => {
                self.album_covers.remove(&album_key(album, artist));
                self.art_key = None;
                self.mark_browser_dirty();
                self.message = format!("saved {}", saved.display());
            }
            Err(e) => self.message = format!("could not save cover.jpg: {e:#}"),
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

    /// Toggle ReplayGain and whether album gain wins over track gain.
    pub fn set_replaygain(&mut self, enabled: bool, album: bool) {
        if self.settings.replaygain == enabled && self.settings.replaygain_album == album {
            return;
        }
        self.settings.replaygain = enabled;
        self.settings.replaygain_album = album;
        if let Some(player) = &self.player {
            player.set_replaygain(enabled, album);
        }
        self.persist_settings();
    }

    /// Overlap the end of one track with the start of the next.
    pub fn set_crossfade(&mut self, secs: f32) {
        let secs = secs.clamp(0.0, 12.0);
        if (self.settings.crossfade_secs - secs).abs() < f32::EPSILON {
            return;
        }
        self.settings.crossfade_secs = secs;
        if let Some(player) = &self.player {
            player.set_crossfade(secs);
        }
        self.persist_settings();
    }

    /// Switch output device; playback continues from the same position.
    pub fn set_output_device(&mut self, device: Option<String>) {
        if self.settings.output_device == device {
            return;
        }
        let previous = self.settings.output_device.clone();
        self.settings.output_device = device;
        if let Err(e) = self.rebuild_player_for_output() {
            self.settings.output_device = previous;
            self.persist_settings();
            self.message = format!("could not change output device: {e}");
            return;
        }
        self.persist_settings();
    }

    fn rebuild_player_for_output(&mut self) -> Result<(), String> {
        let snapshot = self.player.as_ref().map(|player| player.snapshot());
        let path = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.current.clone());
        let position = snapshot.as_ref().map_or(0.0, |snapshot| snapshot.position);
        let was_playing = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.status == musicwasp_core::PlayerStatus::Playing);

        let player = Player::new_with_device(self.settings.output_device.clone())
            .map_err(|e| format!("{e:#}"))?;
        player.set_equalizer(self.settings.equalizer);
        player.set_replaygain(self.settings.replaygain, self.settings.replaygain_album);
        player.set_crossfade(self.settings.crossfade_secs);
        player.set_volume(self.volume);
        if let Some(path) = path {
            player.load_and_play(&path);
            if position > 0.0 {
                player.seek(position);
            }
            if !was_playing {
                player.pause();
            }
        }

        self.player = Some(player);
        self.audio_error = None;
        self.volume_sent = self.volume;
        self.gapless_for = None;
        self.message = match &self.settings.output_device {
            Some(name) => format!("output: {name}"),
            None => "output: system default".to_string(),
        };
        Ok(())
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
        self.finalize_stream();
        self.finish_scrobble();
        if let Err(error) = self.lib.record_play(id) {
            eprintln!("MusicWasp: could not record play for track {id}: {error}");
        }
        if let Some(p) = &self.player {
            p.load_and_play(&path);
        }
        self.gapless_for = None;
        self.selected.clear();
        self.selected.insert(id);
        self.anchor = Some(id);
        self.mark_tracks_dirty();
    }

    pub fn play_from_list(&mut self, index: usize) {
        let target = self.tracks.get(index).map(|track| track.id);
        let current = self.queue_pos.and_then(|pos| self.queue.get(pos).copied());
        if current.is_some() && current != target {
            self.record_current_skip();
        }
        self.queue = self.tracks.iter().map(|t| t.id).collect();
        if self.queue.is_empty() {
            return;
        }
        self.queue_pos = Some(index.min(self.queue.len() - 1));
        self.play_loaded();
    }

    /// Set a star rating from the track table or now-playing view.
    pub fn set_track_rating(&mut self, track_id: i64, rating: u8) {
        if let Err(error) = self.lib.set_rating(track_id, rating) {
            self.message = format!("could not save rating: {error}");
            return;
        }
        if let Some(track) = self.tracks.iter_mut().find(|track| track.id == track_id) {
            track.rating = rating.min(5);
        }
        self.mark_tracks_dirty();
    }

    fn record_current_skip(&mut self) {
        let Some(pos) = self.queue_pos else { return };
        let Some(id) = self.queue.get(pos).copied() else {
            return;
        };
        if let Err(error) = self.lib.record_skip(id) {
            eprintln!("MusicWasp: could not record skip for track {id}: {error}");
        }
        self.mark_tracks_dirty();
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
        if self.queue_pos != Some(index) {
            self.record_current_skip();
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
        self.finalize_stream();
        self.finish_scrobble();
        if let Some(p) = &self.player {
            p.stop();
        }
        self.gapless_for = None;
    }

    pub fn next_track(&mut self) {
        self.next_track_inner(true);
    }

    fn next_track_inner(&mut self, count_skip: bool) {
        let Some(pos) = self.queue_pos else { return };
        if self.queue.is_empty() {
            return;
        }
        if count_skip {
            self.record_current_skip();
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
        self.record_current_skip();
        let prev = if pos > 0 {
            pos - 1
        } else {
            self.queue.len() - 1
        };
        self.queue_pos = Some(prev);
        self.play_loaded();
    }

    /// Index of the track that follows the current one, honouring shuffle/repeat.
    fn next_queue_index(&self) -> Option<usize> {
        let pos = self.queue_pos?;
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        if self.shuffle && len > 1 {
            let stride = 7.min(len - 1).max(1);
            Some((pos + stride) % len)
        } else if pos + 1 < len {
            Some(pos + 1)
        } else if self.repeat != RepeatMode::Off {
            Some(0)
        } else {
            None
        }
    }

    /// Queue the next file once per current track so rodio can cross the file
    /// boundary without the sink draining.
    fn maybe_prime_gapless(&mut self) {
        if self.repeat == RepeatMode::One {
            return;
        }
        let Some(player) = &self.player else { return };
        let snapshot = player.snapshot();
        if snapshot.status != musicwasp_core::PlayerStatus::Playing {
            return;
        }
        // Queue close to the boundary so ordinary queue edits do not leave a
        // stale "next" source waiting in the sink.
        let crossfade = self.settings.crossfade_secs.max(0.0) as f64;
        let window = if crossfade > 0.1 {
            crossfade + 2.0
        } else {
            3.0
        };
        if snapshot.duration > window + 1.0 && snapshot.duration - snapshot.position > window {
            return;
        }
        let Some(pos) = self.queue_pos else { return };
        let Some(current_id) = self.queue.get(pos).copied() else {
            return;
        };
        if self.gapless_for == Some(current_id) {
            return;
        }

        if let Some(next_index) = self.next_queue_index() {
            if let Some(next_id) = self.queue.get(next_index).copied() {
                if let Ok(Some(path)) = self.lib.path_for_id(next_id) {
                    if let Some(player) = &self.player {
                        player.enqueue_next(&path);
                    }
                }
            }
        }
        self.gapless_for = Some(current_id);
    }

    /// Consume gapless track transitions, then end-of-queue events.
    fn poll_player_events(&mut self) {
        let mut advanced = Vec::new();
        if let Some(player) = &self.player {
            while let Some(path) = player.take_advanced() {
                advanced.push(path);
            }
        }

        for path in advanced {
            self.finish_scrobble();
            if let Ok(Some(track)) = self.lib.track_by_path(&path) {
                if let Some(position) = self.queue.iter().position(|id| *id == track.id) {
                    self.queue_pos = Some(position);
                }
                self.playing_id = Some(track.id);
                if let Err(error) = self.lib.record_play(track.id) {
                    eprintln!(
                        "MusicWasp: could not record play for track {}: {error}",
                        track.id
                    );
                }
                self.mark_tracks_dirty();
            }
            self.gapless_for = None;
            self.art_key = None;
        }

        if self
            .player
            .as_ref()
            .is_some_and(|player| player.take_finished())
        {
            self.on_track_finished();
        }
    }

    /// Send one ListenBrainz listen on a worker thread.
    fn send_scrobble(&self, listen: &musicwasp_core::scrobble::Listen, now_playing: bool) {
        let token = self.settings.listenbrainz_token.trim().to_string();
        if token.is_empty() {
            return;
        }
        let listen = listen.clone();
        let tx = self.scrobble_tx.clone();
        std::thread::spawn(move || {
            let result = if now_playing {
                musicwasp_core::scrobble::submit_now_playing(&token, &listen)
            } else {
                musicwasp_core::scrobble::submit_single(&token, &listen)
            };
            if let Err(e) = result {
                let _ = tx.send(format!("ListenBrainz: {e:#}"));
            }
        });
    }

    fn scrobble_listen(track: &Track, started_at: i64) -> musicwasp_core::scrobble::Listen {
        musicwasp_core::scrobble::Listen {
            artist: track.artist.clone(),
            title: track.title.clone(),
            release: track.album.clone(),
            duration_secs: track.duration_secs,
            started_at,
        }
    }

    /// Submit the previous listen if it crossed the ListenBrainz threshold.
    fn finish_scrobble(&mut self) {
        let Some(state) = self.scrobble.take() else {
            return;
        };
        if state.submitted {
            return;
        }
        let duration = state.track.duration_secs as f64;
        let threshold = (duration * 0.5).clamp(1.0, 240.0);
        if state.last_position >= threshold {
            let listen = Self::scrobble_listen(&state.track, state.started_at);
            self.send_scrobble(&listen, false);
        }
    }

    /// Track now-playing and completed listens for ListenBrainz.
    fn update_scrobbling(&mut self) {
        while let Ok(error) = self.scrobble_rx.try_recv() {
            self.message = error;
        }
        if !self.settings.scrobble_listenbrainz
            || self.settings.listenbrainz_token.trim().is_empty()
        {
            self.scrobble = None;
            return;
        }
        let Some(player) = &self.player else {
            self.scrobble = None;
            return;
        };
        let snapshot = player.snapshot();
        let current = self
            .queue_pos
            .and_then(|pos| self.queue.get(pos).copied())
            .and_then(|id| self.lib.tracks_by_ids(&[id]).ok())
            .and_then(|mut tracks| tracks.pop());
        let Some(track) = current else {
            self.finish_scrobble();
            return;
        };
        if snapshot.status == musicwasp_core::PlayerStatus::Stopped {
            self.finish_scrobble();
            return;
        }

        let changed = self
            .scrobble
            .as_ref()
            .is_none_or(|state| state.track.id != track.id);
        if changed {
            self.finish_scrobble();
            if track.title.trim().is_empty() || track.artist.trim().is_empty() {
                return;
            }
            let started_at = musicwasp_core::scrobble::now();
            let listen = Self::scrobble_listen(&track, started_at);
            self.scrobble = Some(ScrobbleState {
                track,
                started_at,
                submitted: false,
                last_position: snapshot.position,
            });
            self.send_scrobble(&listen, true);
            return;
        }

        let mut completed = None;
        if let Some(state) = &mut self.scrobble {
            state.last_position = snapshot.position;
            let duration = state.track.duration_secs as f64;
            let threshold = (duration * 0.5).clamp(1.0, 240.0);
            if !state.submitted && snapshot.position >= threshold {
                state.submitted = true;
                completed = Some(Self::scrobble_listen(&state.track, state.started_at));
            }
        }
        if let Some(listen) = completed {
            self.send_scrobble(&listen, false);
        }
    }

    pub fn on_track_finished(&mut self) {
        if self.stream.info.is_some() {
            self.save_stream_progress(true);
            self.stream.info = None;
            return;
        }
        match self.repeat {
            RepeatMode::One => self.play_loaded(),
            _ => self.next_track_inner(false),
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
        if let Err(e) = self.settings.save_persisted(&self.settings_path) {
            self.message = format!("could not save settings: {e}");
        }
    }

    /// Restore every setting to its default and apply the ones that are live
    /// (theme, window icon, folder watcher).
    pub fn reset_settings(&mut self, ctx: &egui::Context) {
        let had_output_device = self.settings.output_device.is_some();
        self.settings = Settings::default();
        self.capture = None;
        self.watch_dirty = true;
        crate::i18n::set_language(self.settings.language);
        crate::theme::apply(ctx, self.settings.skin);
        crate::theme::apply_text_scale(ctx, self.settings.large_fonts);
        self.refresh_brand_icon(ctx);
        if let Some(player) = &self.player {
            player.set_replaygain(self.settings.replaygain, self.settings.replaygain_album);
            player.set_crossfade(self.settings.crossfade_secs);
        }
        if had_output_device {
            let _ = self.rebuild_player_for_output();
        }
        self.persist_settings();
    }

    // ---- library backups ----

    /// Write a timestamped backup of the current database.
    pub fn backup_library(&mut self) -> Result<PathBuf, String> {
        if self.scan.is_some() {
            return Err("stop indexing before backing up the library".to_string());
        }
        let path = self
            .lib
            .create_backup()
            .map_err(|error| error.to_string())?;
        self.message = format!("library backed up to {}", path.display());
        if let Some(draft) = &mut self.restore_dialog {
            draft.backups = musicwasp_core::backup::list().unwrap_or_default();
        }
        Ok(path)
    }

    /// Open the restore dialog with the available backups.
    pub fn open_restore_dialog(&mut self) {
        let backups = musicwasp_core::backup::list().unwrap_or_default();
        let path = backups
            .first()
            .map(|backup| backup.path.to_string_lossy().to_string())
            .unwrap_or_default();
        self.restore_dialog = Some(RestoreDraft {
            path,
            backups,
            confirm: false,
            status: None,
        });
    }

    /// Replace the library with a validated backup. Refuses while scanning.
    pub fn restore_library(&mut self, path: &Path) -> Result<String, String> {
        if self.scan.is_some() {
            return Err("stop indexing before restoring the library".to_string());
        }
        let safety = self
            .lib
            .restore_from(path)
            .map_err(|error| format!("{error:#}"))?;

        if let Some(player) = &self.player {
            player.stop();
        }
        self.queue.clear();
        self.queue_pos = None;
        self.playing_id = None;
        self.art = None;
        self.art_key = None;
        self.gapless_for = None;
        self.selected.clear();
        self.anchor = None;
        self.view = View::Library;
        self.album_view = false;
        self.hygiene = None;
        self.show_hygiene = false;
        self.organize = None;
        self.show_organize = false;
        self.smart_dialog = None;
        self.playlist_dialog = None;
        self.m3u_dialog = None;
        self.mark_browser_dirty();
        self.mark_tracks_dirty();
        self.refresh_browser();
        self.refresh_tracks();

        let message = format!(
            "library restored from {}; safety backup: {}",
            path.display(),
            safety.display()
        );
        self.message = message.clone();
        Ok(message)
    }

    // ---- mini player and tray ----

    /// Open or close the compact always-on-top player.
    pub fn set_mini_player(&mut self, enabled: bool, ctx: &egui::Context) {
        if self.settings.mini_player == enabled {
            return;
        }
        self.settings.mini_player = enabled;
        if !enabled {
            self.mini = None;
        }
        self.persist_settings();
        self.message = if enabled {
            "mini player opened".to_string()
        } else {
            "mini player closed".to_string()
        };
        ctx.request_repaint();
    }

    /// Enable or disable the Linux tray icon.
    #[cfg(target_os = "linux")]
    pub fn set_tray_enabled(&mut self, enabled: bool, ctx: &egui::Context) {
        if self.settings.tray_enabled == enabled {
            return;
        }
        self.settings.tray_enabled = enabled;
        if enabled {
            match crate::tray::TrayController::spawn(ctx) {
                Ok(tray) => self.tray = Some(tray),
                Err(error) => {
                    self.settings.tray_enabled = false;
                    self.message = format!("tray unavailable: {error}");
                }
            }
        } else {
            self.tray = None;
        }
        self.persist_settings();
        ctx.request_repaint();
    }

    fn update_mini_player(&mut self, ctx: &egui::Context) {
        if !self.settings.mini_player {
            self.mini = None;
            return;
        }
        let snapshot = self.player.as_ref().map(|player| player.snapshot());
        let current_path = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.current.as_deref());
        let track = self.current_track(current_path);
        let playing = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.status == musicwasp_core::PlayerStatus::Playing);
        let stream = self.stream.info.clone();
        let duration = stream
            .as_ref()
            .map(|stream| stream.duration_secs as f64)
            .filter(|duration| *duration > 0.0)
            .or_else(|| track.as_ref().map(|track| track.duration_secs as f64))
            .or_else(|| snapshot.as_ref().map(|snapshot| snapshot.duration))
            .unwrap_or(0.0);
        let bridge = self
            .mini
            .get_or_insert_with(crate::mini_player::MiniBridge::new);
        bridge.update(crate::mini_player::MiniSnapshot {
            title: stream
                .as_ref()
                .map(|stream| stream.title.clone())
                .or_else(|| track.as_ref().map(|track| track.title.clone()))
                .unwrap_or_default(),
            artist: stream
                .as_ref()
                .map(|stream| stream.subtitle.clone())
                .or_else(|| track.as_ref().map(|track| track.artist.clone()))
                .unwrap_or_default(),
            album: stream
                .as_ref()
                .map(|_| String::new())
                .or_else(|| track.as_ref().map(|track| track.album.clone()))
                .unwrap_or_default(),
            playing,
            position: snapshot.as_ref().map_or(0.0, |snapshot| snapshot.position),
            duration,
        });

        let shared = bridge.shared();
        let root = ctx.clone();
        let skin = self.settings.skin;
        ctx.show_viewport_deferred(
            crate::mini_player::MiniBridge::viewport_id(),
            crate::mini_player::builder(),
            move |ui, _class| crate::mini_player::ui(ui, &shared, &root, skin),
        );

        let (commands, close) = match &self.mini {
            Some(bridge) => (bridge.take_commands(), bridge.close_requested()),
            None => (Vec::new(), false),
        };
        for command in commands {
            self.apply_mini_command(ctx, command);
        }
        if close {
            self.set_mini_player(false, ctx);
        }
    }

    fn apply_mini_command(
        &mut self,
        ctx: &egui::Context,
        command: crate::mini_player::MiniCommand,
    ) {
        use crate::mini_player::MiniCommand;
        match command {
            MiniCommand::PlayPause => self.toggle_pause(),
            MiniCommand::Next => self.next_track(),
            MiniCommand::Previous => self.prev_track(),
            MiniCommand::Seek(seconds) => {
                if let Some(player) = &self.player {
                    player.seek(seconds);
                }
            }
            MiniCommand::ShowMain => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            MiniCommand::Close => self.set_mini_player(false, ctx),
        }
    }

    #[cfg(target_os = "linux")]
    fn update_tray(&mut self, ctx: &egui::Context) {
        if !self.settings.tray_enabled {
            self.tray = None;
            return;
        }
        if self.tray.is_none() {
            match crate::tray::TrayController::spawn(ctx) {
                Ok(tray) => self.tray = Some(tray),
                Err(error) => {
                    self.settings.tray_enabled = false;
                    self.persist_settings();
                    self.message = format!("tray unavailable: {error}");
                    return;
                }
            }
        }
        let track = self
            .playing_id
            .and_then(|id| self.lib.tracks_by_ids(&[id]).ok())
            .and_then(|mut tracks| tracks.pop());
        let playing = self.player.as_ref().is_some_and(|player| {
            player.snapshot().status == musicwasp_core::PlayerStatus::Playing
        });
        let (title, artist) = if let Some(stream) = &self.stream.info {
            (stream.title.as_str(), stream.subtitle.as_str())
        } else {
            (
                track.as_ref().map_or("", |track| track.title.as_str()),
                track.as_ref().map_or("", |track| track.artist.as_str()),
            )
        };
        if let Some(tray) = &self.tray {
            tray.update(title, artist, playing, self.settings.mini_player);
        }
        let commands = self
            .tray
            .as_ref()
            .map(crate::tray::TrayController::drain)
            .unwrap_or_default();
        for command in commands {
            self.apply_tray_command(ctx, command);
        }
    }

    #[cfg(target_os = "linux")]
    fn apply_tray_command(&mut self, ctx: &egui::Context, command: crate::tray::Command) {
        use crate::tray::Command;
        match command {
            Command::PlayPause => self.toggle_pause(),
            Command::Next => self.next_track(),
            Command::Previous => self.prev_track(),
            Command::Show => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            Command::ToggleMini => self.set_mini_player(!self.settings.mini_player, ctx),
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    // ---- skin editing ----

    /// Open the palette editor for the current skin.
    pub fn open_skin_editor(&mut self) {
        let skin = self.settings.skin;
        self.skin_editor = Some(SkinEditorState {
            skin,
            palette: skin.palette(),
            status: None,
        });
        self.show_skin_editor = true;
    }

    /// Apply an edited palette without persisting it yet.
    pub fn preview_skin_palette(&mut self, ctx: &egui::Context, palette: crate::theme::Palette) {
        if let Some(editor) = &self.skin_editor {
            crate::theme::preview(ctx, editor.skin, palette);
        }
    }

    /// Write the edited palette to `skins.json` and keep it selected.
    pub fn save_skin_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.skin_editor else {
            return;
        };
        match crate::theme::save_palette(editor.skin, editor.palette) {
            Ok(skin) => {
                self.settings.skin = skin;
                crate::theme::apply(ctx, skin);
                self.refresh_brand_icon(ctx);
                self.persist_settings();
                self.skin_editor = None;
                self.show_skin_editor = false;
                self.message = "skin colours saved".to_string();
            }
            Err(error) => {
                if let Some(editor) = &mut self.skin_editor {
                    editor.status = Some(format!("save failed: {error}"));
                }
            }
        }
    }

    /// Drop the live preview and restore the stored palette.
    pub fn cancel_skin_editor(&mut self, ctx: &egui::Context) {
        if let Some(editor) = &self.skin_editor {
            crate::theme::apply(ctx, editor.skin);
        }
        self.skin_editor = None;
        self.show_skin_editor = false;
    }

    /// Re-read `skins.json` and re-apply the selected skin.
    pub fn reload_skins(&mut self, ctx: &egui::Context) {
        crate::theme::load_user_skins();
        let skin = crate::theme::Skin::from_id(self.settings.skin.id());
        self.settings.skin = skin;
        crate::theme::apply(ctx, skin);
        self.refresh_brand_icon(ctx);
        self.persist_settings();
        self.skin_editor = None;
        self.show_skin_editor = false;
        self.message = "skins reloaded".to_string();
    }

    /// Copy the selected skin and switch to the new one.
    pub fn duplicate_skin(&mut self, ctx: &egui::Context) {
        match crate::theme::duplicate(self.settings.skin) {
            Ok(skin) => {
                self.settings.skin = skin;
                crate::theme::apply(ctx, skin);
                self.refresh_brand_icon(ctx);
                self.persist_settings();
                self.message = format!("duplicated skin \"{}\"", skin.label());
                self.open_skin_editor();
            }
            Err(error) => self.message = format!("could not duplicate skin: {error}"),
        }
    }

    /// Reveal the data directory that holds `skins.json`.
    pub fn open_skins_folder(&mut self) {
        match crate::theme::skins_folder() {
            Ok(folder) => {
                let file = folder.join("skins.json");
                if !file.exists() {
                    let _ = std::fs::write(&file, "{\n  \"skins\": []\n}\n");
                }
                match reveal_folder(&folder) {
                    Ok(()) => self.message = format!("opened {}", folder.display()),
                    Err(error) => {
                        self.message = format!("could not open skins folder: {error}");
                    }
                }
            }
            Err(error) => self.message = format!("could not open skins folder: {error}"),
        }
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

    /// Switch UI language immediately and remember the choice.
    pub fn set_language(&mut self, language: Language) {
        if self.settings.language != language {
            self.settings.language = language;
            crate::i18n::set_language(language);
            self.persist_settings();
        }
    }

    /// Toggle the larger-text accessibility mode.
    pub fn set_large_fonts(&mut self, ctx: &egui::Context, large: bool) {
        if self.settings.large_fonts != large {
            self.settings.large_fonts = large;
            crate::theme::apply_text_scale(ctx, large);
            self.persist_settings();
        }
    }

    /// Swap the toolbar logo and window icon to match the active theme.
    fn refresh_brand_icon(&mut self, ctx: &egui::Context) {
        let skin = self.settings.skin;
        self.logo = crate::icons::logo(ctx, skin, 64);
        if let Some(icon) = crate::icons::icon_data(skin) {
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
            if let Ok(img) = crate::images::from_bytes(&bytes) {
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
            let track = &tracks[0];
            (
                musicwasp_core::tags::read(Path::new(&track.path))
                    .unwrap_or_else(|_| TagEdit::from_track(track)),
                false,
            )
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

    /// Snapshot files immediately before a tag write, so it can be undone.
    fn store_tag_undo(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        let snapshots = musicwasp_core::tags::snapshot_paths(paths)
            .map_err(|e| format!("cannot snapshot tags: {e:#}"))?;
        musicwasp_core::tags::save_undo(&self.tag_undo_path, &snapshots)
            .map_err(|e| format!("cannot store undo snapshot: {e:#}"))?;
        self.tag_undo = Some(snapshots);
        Ok(())
    }

    /// Restore the tags captured before the most recent write.
    pub fn undo_last_tag_write(&mut self) {
        let Some(snapshots) = self.tag_undo.clone() else {
            return;
        };

        let mut restored = 0usize;
        let mut failed = Vec::new();
        let mut errors = Vec::new();
        for snapshot in snapshots {
            match musicwasp_core::tags::restore(&snapshot) {
                Ok(()) => {
                    restored += 1;
                    if let Err(e) = self.lib.refresh_file(&snapshot.path) {
                        errors.push(format!("{}: reindex failed: {e}", snapshot.path.display()));
                    }
                }
                Err(e) => {
                    errors.push(format!("{}: {e:#}", snapshot.path.display()));
                    failed.push(snapshot);
                }
            }
        }

        if failed.is_empty() {
            musicwasp_core::tags::clear_undo(&self.tag_undo_path);
            self.tag_undo = None;
        } else {
            let _ = musicwasp_core::tags::save_undo(&self.tag_undo_path, &failed);
            self.tag_undo = Some(failed);
        }

        self.art_key = None;
        self.mark_browser_dirty();
        self.mark_tracks_dirty();

        let mut summary = format!("restored tags for {restored} file(s)");
        if !errors.is_empty() {
            summary.push_str(&format!(", {} failed ({} first)", errors.len(), errors[0]));
        }
        self.message = summary;
    }

    /// Apply an immediate per-file transformation to the open batch selection.
    pub fn apply_batch_action(&mut self, action: BatchAction) {
        let Some(state) = &self.editing else {
            return;
        };
        if !state.batch {
            return;
        }
        let paths = state.paths.clone();
        if paths.is_empty() {
            return;
        }
        if let Err(e) = self.store_tag_undo(&paths) {
            if let Some(state) = &mut self.editing {
                state.status = Some(format!("nothing written: {e}"));
            }
            return;
        }

        let mut edits = Vec::with_capacity(paths.len());
        for path in &paths {
            match musicwasp_core::tags::read(path) {
                Ok(edit) => edits.push(edit),
                Err(e) => {
                    if let Some(state) = &mut self.editing {
                        state.status = Some(format!(
                            "nothing written: cannot read {}: {e:#}",
                            path.display()
                        ));
                    }
                    return;
                }
            }
        }

        match action {
            BatchAction::AutoNumber => {
                auto_number_tracks(&mut edits);
            }
            BatchAction::Case(style) => {
                for edit in &mut edits {
                    for value in [
                        &mut edit.title,
                        &mut edit.artist,
                        &mut edit.album,
                        &mut edit.album_artist,
                        &mut edit.genre,
                    ] {
                        *value = normalize_case_value(value, style);
                    }
                }
            }
            BatchAction::Clear(field) => {
                for edit in &mut edits {
                    field.clear(edit);
                }
            }
            BatchAction::Copy(field) => {
                let source = field.value(&edits[0]);
                for edit in edits.iter_mut().skip(1) {
                    field.set(edit, &source);
                }
            }
        }

        let mut written = 0usize;
        let mut errors = Vec::new();
        for (path, edit) in paths.iter().zip(edits.iter()) {
            match musicwasp_core::tags::write(path, edit) {
                Ok(()) => {
                    if let Err(e) = self.lib.refresh_file(path) {
                        errors.push(format!("{}: reindex failed: {e}", path.display()));
                    } else {
                        written += 1;
                    }
                }
                Err(e) => errors.push(format!("{}: {e:#}", path.display())),
            }
        }

        self.art_key = None;
        self.mark_browser_dirty();
        let mut summary = format!("{}: updated {written} file(s)", action.label());
        if !errors.is_empty() {
            summary.push_str(&format!(", {} failed ({})", errors.len(), errors[0]));
        }
        self.message = summary.clone();
        if let Some(state) = &mut self.editing {
            state.status = Some(summary);
        }
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
        let paths = state.paths.clone();
        if let Err(e) = self.store_tag_undo(&paths) {
            if let Some(state) = &mut self.editing {
                state.status = Some(format!("nothing written: {e}"));
            }
            return;
        }
        let Some(state) = &mut self.editing else {
            return;
        };

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
    /// the pointer, or into the whole library (tr("ui.all_music")).
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

        // Dropping M3U files imports them wherever the pointer is; they are
        // playlists, not audio to add to the row under the cursor.
        let mut notes = Vec::new();
        let mut audio_paths = Vec::new();
        for path in paths {
            if musicwasp_core::m3u::is_m3u_path(&path) {
                match self.import_m3u_file(&path, None) {
                    Ok(summary) => notes.push(summary),
                    Err(error) => notes.push(format!("{}: {error}", path.display())),
                }
            } else {
                audio_paths.push(path);
            }
        }
        if !notes.is_empty() {
            self.message = notes.join("; ");
            ctx.request_repaint();
        }
        if audio_paths.is_empty() {
            self.os_drop_hover = None;
            return;
        }
        let paths = audio_paths;

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
                View::Library | View::Podcasts | View::Radio => {
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
            RenameTarget::Folder(id) => self
                .playlist_folders
                .iter()
                .find(|folder| folder.id == *id)
                .map(|folder| folder.name.clone())
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
                        let path = PathBuf::from(&track.path);
                        self.store_tag_undo(std::slice::from_ref(&path))
                            .and_then(|()| {
                                let mut edit = musicwasp_core::tags::read(&path)
                                    .unwrap_or_else(|_| TagEdit::from_track(&track));
                                field.apply(&mut edit, &text)?;
                                musicwasp_core::tags::write(&path, &edit)
                                    .map_err(|e| e.to_string())?;
                                self.lib
                                    .refresh_file(&path)
                                    .map_err(|e| format!("reindex failed: {e}"))
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
            RenameTarget::Folder(id) => {
                if text.is_empty() {
                    Err("the new name cannot be empty".to_string())
                } else {
                    self.lib
                        .rename_playlist_folder(*id, &text)
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
                    RenameTarget::Folder(_) => "folder renamed".to_string(),
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

    /// User-entered key, or the key baked in at build time.
    pub fn effective_acoustid_key(&self) -> String {
        musicwasp_core::autotag::acoustid_key()
    }

    /// Identify the selected album from one audio fingerprint, then load the
    /// first matching release exactly like a text search would.
    pub fn start_autotag_fingerprint(&mut self) {
        let key = self.effective_acoustid_key();
        let Some(state) = &mut self.autotag else {
            return;
        };
        if key.is_empty() {
            state.stage = AutoTagStage::Done(
                "this build has no AcoustID key; set ACCOUSTID_APIKEY and rebuild".to_string(),
            );
            return;
        }
        let Some(target) = state.targets.iter().max_by_key(|track| track.duration_secs) else {
            state.stage = AutoTagStage::Done("no tracks selected".to_string());
            return;
        };
        let path = PathBuf::from(&target.path);
        state.stage = AutoTagStage::Fingerprinting;
        let tx = state.tx.clone();
        std::thread::spawn(move || {
            let result = autotag::fingerprint(&path)
                .and_then(|fingerprint| {
                    autotag::acoustid_lookup(&key, &fingerprint.fingerprint, fingerprint.duration)
                })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(AutoTagMsg::Fingerprinted(result));
        });
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
        let undo_path = self.tag_undo_path.clone();
        if let Some(state) = &mut self.autotag {
            state.stage = AutoTagStage::Applying;
        }

        std::thread::spawn(move || {
            let outcome = (|| -> anyhow::Result<(usize, Vec<String>, Vec<PathBuf>, bool)> {
                // Capture and persist the old tags before touching anything. If
                // this fails, do not write: a failed rollback is worse than no
                // auto-tag.
                let paths: Vec<PathBuf> = matches.iter().map(|m| m.path.clone()).collect();
                let snapshots = musicwasp_core::tags::snapshot_paths(&paths)?;
                musicwasp_core::tags::save_undo(&undo_path, &snapshots)?;

                let cover = if embed_art {
                    autotag::front_cover(&candidate.release_id).ok().flatten()
                } else {
                    None
                };
                let (written, errors) = autotag::apply(&release, &matches, cover.as_ref());
                Ok((written, errors, paths, cover.is_some()))
            })();

            let message = match outcome {
                Ok((written, errors, paths, cover)) => AutoTagMsg::Applied {
                    written,
                    errors,
                    paths,
                    cover,
                    cover_requested: embed_art,
                },
                Err(e) => AutoTagMsg::Failed(format!("{e:#}")),
            };
            let _ = tx.send(message);
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
                AutoTagMsg::Fingerprinted(Ok(candidates)) => {
                    if candidates.is_empty() {
                        if let Some(state) = &mut self.autotag {
                            state.stage = AutoTagStage::Done(
                                "AcoustID found no release for this fingerprint".to_string(),
                            );
                        }
                        continue;
                    }
                    let local_count = self.autotag_targets().len();
                    if let Some(state) = &mut self.autotag {
                        let mut ranked = candidates;
                        ranked.sort_by_key(|candidate| -autotag::relevance(candidate, local_count));
                        state.selected = 0;
                        state.stage = AutoTagStage::Results(ranked);
                        state.release = None;
                        state.release_id = None;
                        state.release_error = None;
                        state.loading_release = false;
                        state.mappings.clear();
                    }
                    self.load_autotag_release();
                }
                AutoTagMsg::Fingerprinted(Err(e)) => {
                    if let Some(state) = &mut self.autotag {
                        state.stage = AutoTagStage::Done(format!("AcoustID: {e}"));
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
                    self.tag_undo = musicwasp_core::tags::load_undo(&self.tag_undo_path)
                        .ok()
                        .flatten();
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
                AutoTagMsg::Failed(e) => {
                    let summary = format!("failed: {e}");
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
        let path = PathBuf::from(self.scan_path.trim());
        if self.scan.is_some() {
            self.message = "indexing is already running".to_string();
            return;
        }
        if !path.is_dir() {
            self.message = format!("not a folder: {}", path.display());
            return;
        }
        let db_path = match self.lib.db_path() {
            Ok(path) => path,
            Err(e) => {
                self.message = format!("scan failed: {e}");
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let root = path.clone();
        self.scan = Some(ScanState {
            root: path.clone(),
            progress: ScanProgress::default(),
            cancel,
            rx,
            last_refresh: std::time::Instant::now(),
        });
        self.message = format!("indexing {} in the background…", path.display());

        std::thread::spawn(move || {
            let outcome = (|| -> anyhow::Result<ScanReport> {
                let library = Library::open_at(&db_path)?;
                let mut last_emit = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or_else(std::time::Instant::now);
                library.scan_with_progress(&root, false, |progress| {
                    if last_emit.elapsed() >= std::time::Duration::from_millis(120) {
                        let _ = tx.send(ScanMsg::Progress(progress));
                        last_emit = std::time::Instant::now();
                    }
                    !worker_cancel.load(Ordering::Relaxed)
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(ScanMsg::Finished(outcome));
        });
    }

    pub fn is_scanning(&self) -> bool {
        self.scan.is_some()
    }

    pub fn scan_progress(&self) -> Option<&ScanProgress> {
        self.scan.as_ref().map(|state| &state.progress)
    }

    pub fn scan_progress_label(&self) -> Option<String> {
        self.scan.as_ref().map(|state| {
            let progress = &state.progress;
            if progress.total == 0 {
                format!("{}: counting files…", state.root.display())
            } else {
                format!(
                    "{}: {} / {} files, {} indexed, {} unchanged",
                    state.root.display(),
                    progress.found,
                    progress.total,
                    progress.indexed,
                    progress.unchanged
                )
            }
        })
    }

    pub fn cancel_scan(&mut self) {
        if let Some(state) = &self.scan {
            state.cancel.store(true, Ordering::Relaxed);
            self.message = "stopping indexing after the current file…".to_string();
        }
    }

    fn poll_scan(&mut self) {
        let mut latest = None;
        let mut finished = None;
        let mut disconnected = false;
        let mut refresh = false;
        let mut root = None;

        if let Some(state) = &mut self.scan {
            loop {
                match state.rx.try_recv() {
                    Ok(ScanMsg::Progress(progress)) => {
                        state.progress = progress.clone();
                        latest = Some(progress);
                    }
                    Ok(ScanMsg::Finished(result)) => finished = Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            root = Some(state.root.clone());
            if latest.is_some() && state.last_refresh.elapsed() >= std::time::Duration::from_secs(1)
            {
                state.last_refresh = std::time::Instant::now();
                refresh = true;
            }
        }

        if let Some(progress) = &latest {
            if let Some(root) = &root {
                self.message = if progress.total == 0 {
                    format!("indexing {}: counting files…", root.display())
                } else {
                    format!(
                        "indexing {}: {}/{} files, {} indexed, {} unchanged",
                        root.display(),
                        progress.found,
                        progress.total,
                        progress.indexed,
                        progress.unchanged
                    )
                };
            }
        }
        if refresh {
            self.mark_browser_dirty();
            self.mark_tracks_dirty();
        }

        if let Some(result) = finished {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            self.scan = None;
            match result {
                Ok(report) if report.cancelled => {
                    self.message = format!(
                        "indexing stopped after {}/{} file(s), {} indexed",
                        report.found, report.total, report.indexed
                    );
                }
                Ok(report) => {
                    self.remember_watch_folder(&root);
                    self.message = format!(
                        "scanned {} track(s) from {}: {} new/updated, {} unchanged, {} failed",
                        report.found,
                        root.display(),
                        report.indexed,
                        report.unchanged,
                        report.failed
                    );
                }
                Err(e) => self.message = format!("scan failed: {e}"),
            }
            self.mark_browser_dirty();
            self.mark_tracks_dirty();
        } else if disconnected {
            self.scan = None;
            self.message = "indexing worker stopped unexpectedly".to_string();
            self.mark_browser_dirty();
            self.mark_tracks_dirty();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.album_covers.pump(&ctx);
        crate::theme::paint_backdrop(root, self.settings.skin);
        self.capture_window(&ctx);
        if self.watch_dirty {
            self.restart_watcher(&ctx);
            self.watch_dirty = false;
        }
        self.poll_watcher(&ctx);
        self.poll_scan();
        if self.is_scanning() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        #[cfg(target_os = "linux")]
        self.poll_mpris(&ctx);
        self.poll_cover_fetch();
        self.poll_autotag();
        self.handle_capture(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_screenshot_capture(&ctx);

        self.poll_player_events();
        self.sync_lyrics();
        self.poll_podcasts();
        self.update_stream_progress();
        self.update_scrobbling();
        self.maybe_prime_gapless();
        self.sync_volume();
        self.sync_equalizer();
        self.update_cover_art(&ctx);
        self.update_visualizer(&ctx);

        if self.viz_fullscreen
            && self.settings.visualizer.enabled
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.viz_fullscreen = false;
        }
        if self.settings.visualizer.enabled && ctx.input(|input| input.key_pressed(egui::Key::F11))
        {
            self.viz_fullscreen = !self.viz_fullscreen;
        }

        if self.dirty_browser {
            self.refresh_browser();
        }
        if self.dirty_tracks {
            self.refresh_tracks();
        }
        if self.album_view && self.albums_dirty {
            self.refresh_album_groups();
        }

        if self.viz_fullscreen && self.settings.visualizer.enabled {
            crate::ui::visualizer_fullscreen(self, root);
        } else {
            crate::ui::toolbar(self, root);
            if self.plain_view {
                self.os_drop_zones.clear();
                crate::ui::lyrics_panel(self, root);
                crate::ui::now_playing(self, root);
            } else if self.view == View::Podcasts {
                crate::ui::transport(self, root);
                crate::ui::podcasts_view(self, root);
            } else if self.view == View::Radio {
                crate::ui::transport(self, root);
                crate::ui::radio_view(self, root);
            } else {
                crate::ui::browser(self, root);
                crate::ui::transport(self, root);
                crate::ui::visualizer_panel(self, root);
                crate::ui::queue_panel(self, root);
                crate::ui::lyrics_panel(self, root);
                if self.album_view {
                    crate::ui::album_grid(self, root);
                } else {
                    crate::ui::tracklist(self, root);
                }
            }
        }
        // Handle OS file drops after the browser has registered this frame's
        // playlist rows, so the pointer maps to the row under it.
        self.handle_os_drop(&ctx);
        crate::ui::tag_editor(self, &ctx);
        crate::ui::playlist_dialog(self, &ctx);
        crate::ui::folder_dialog(self, &ctx);
        crate::ui::m3u_dialog(self, &ctx);
        crate::ui::saved_search_dialog(self, &ctx);
        crate::ui::columns_window(self, &ctx);
        crate::ui::smart_dialog(self, &ctx);
        crate::ui::settings_window(self, &ctx);
        crate::hygiene::window(self, &ctx);
        crate::organize::window(self, &ctx);
        crate::skin_editor::window(self, &ctx);
        crate::backup_dialog::window(self, &ctx);
        self.update_mini_player(&ctx);
        crate::ui::cover_picker(self, &ctx);
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
        {
            self.update_tray(&ctx);
            self.publish_mpris(&ctx);
        }

        self.persist_session(false);
    }

    fn on_exit(&mut self) {
        self.save_stream_progress(false);
        if let Some(state) = &self.scan {
            state.cancel.store(true, Ordering::Relaxed);
        }
        self.persist_session(true);
    }
}

impl App {
    pub fn scan_clicked(&mut self) {
        self.scan();
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
#[path = "../tests/unit/app.rs"]
mod tests;

/// Open a folder in the desktop file manager.
fn reveal_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let command = "xdg-open";
    #[cfg(target_os = "windows")]
    let command = "explorer";
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let command = "xdg-open";

    std::process::Command::new(command)
        .arg(path)
        .spawn()
        .map(|_| ())
}
