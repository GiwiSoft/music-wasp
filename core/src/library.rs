use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{Accessor, AudioFile, ItemKey};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagExt};
use rusqlite::{params, params_from_iter, types::Value, Connection};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Owns the SQLite connection and exposes every library operation the UI needs.
pub struct Library {
    conn: Connection,
    /// File this library was opened from, for diagnostics and tests.
    path: Option<PathBuf>,
}

impl Library {
    pub fn open() -> Result<Self> {
        Self::open_at(&crate::db::path()?)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        Ok(Self {
            conn: crate::db::open_at(path)?,
            path: Some(path.to_path_buf()),
        })
    }

    pub fn query(&self, filter: &Filter, sort: SortKey, asc: bool) -> Result<Vec<Track>> {
        query(&self.conn, filter, sort, asc)
    }

    pub fn genres(&self) -> Result<Vec<String>> {
        genres(&self.conn)
    }

    pub fn artists(&self, genre: Option<&str>) -> Result<Vec<String>> {
        artists(&self.conn, genre)
    }

    pub fn albums(&self, artist: Option<&str>, genre: Option<&str>) -> Result<Vec<String>> {
        albums(&self.conn, artist, genre)
    }

    pub fn path_for_id(&self, id: i64) -> Result<Option<PathBuf>> {
        path_for_id(&self.conn, id)
    }

    /// Tracks for a set of ids, in the order requested. Missing ids are skipped.
    pub fn tracks_by_ids(&self, ids: &[i64]) -> Result<Vec<Track>> {
        tracks_by_ids(&self.conn, ids)
    }

    pub fn track_by_path(&self, path: &Path) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1"
        ))?;
        let mut rows = stmt.query(rusqlite::params![path.to_string_lossy()])?;
        Ok(match rows.next()? {
            Some(row) => Some(row_to_track(row)?),
            None => None,
        })
    }

    pub fn stats(&self) -> Result<Stats> {
        stats(&self.conn)
    }

    /// Tracks whose files are no longer on disk.
    pub fn missing_tracks(&self) -> Result<Vec<Track>> {
        missing_tracks(&self.conn)
    }

    /// Read-only preview of vanished rows matched to files still on disk.
    pub fn reconcile(&self, roots: &[PathBuf]) -> Result<crate::reconcile::ReconcilePlan> {
        crate::reconcile::reconcile(&self.conn, roots)
    }

    /// Apply accepted reconciliation candidates. Files are never modified.
    pub fn apply_reconcile(
        &self,
        candidates: &[crate::reconcile::ReconcileCandidate],
    ) -> Result<crate::reconcile::ReconcileOutcome> {
        crate::reconcile::apply(&self.conn, candidates)
    }

    /// Albums for the cover grid, grouped by album and album artist.
    pub fn album_groups(&self) -> Result<Vec<AlbumGroup>> {
        album_groups(&self.conn)
    }

    /// All tracks of one album, used by the cover tools.
    pub fn album_tracks(&self, album: &str, artist: &str) -> Result<Vec<Track>> {
        album_tracks(&self.conn, album, artist)
    }

    /// Groups of tracks sharing normalized artist, title and album.
    pub fn duplicate_groups(&self) -> Result<Vec<Vec<Track>>> {
        duplicate_groups(&self.conn)
    }

    /// Delete library rows only. Audio files stay on disk, and playlist entries
    /// disappear through the `ON DELETE CASCADE` foreign keys.
    pub fn remove_tracks(&self, ids: &[i64]) -> Result<usize> {
        remove_tracks(&self.conn, ids)
    }

    /// Set a 0..5 star rating for one track.
    pub fn set_rating(&self, track_id: i64, rating: u8) -> Result<()> {
        set_rating(&self.conn, track_id, rating)
    }

    /// Record a started/completed play and update `last_played`.
    pub fn record_play(&self, track_id: i64) -> Result<()> {
        record_play(&self.conn, track_id)
    }

    pub fn record_skip(&self, track_id: i64) -> Result<()> {
        record_skip(&self.conn, track_id)
    }

    pub fn scan(&self, root: &Path, rescan: bool) -> Result<usize> {
        scan(&self.conn, root, rescan)
    }

    /// Incremental scan with progress and cancellation. Return `false` from
    /// `progress` to stop after the current file; committed batches are kept.
    pub fn scan_with_progress<F>(
        &self,
        root: &Path,
        rescan: bool,
        progress: F,
    ) -> Result<ScanReport>
    where
        F: FnMut(ScanProgress) -> bool,
    {
        scan_with_progress(&self.conn, root, rescan, progress)
    }

    pub fn playlists(&self) -> Result<Vec<crate::playlist::Playlist>> {
        crate::playlist::list(&self.conn)
    }

    pub fn create_playlist(&self, name: &str, track_ids: &[i64]) -> Result<i64> {
        crate::playlist::create(&self.conn, name, track_ids)
    }

    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {
        crate::playlist::tracks(&self.conn, playlist_id)
    }

    pub fn add_to_playlist(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        crate::playlist::add_track(&self.conn, playlist_id, track_id)
    }

    pub fn add_many_to_playlist(&self, playlist_id: i64, track_ids: &[i64]) -> Result<usize> {
        crate::playlist::add_tracks(&self.conn, playlist_id, track_ids)
    }

    pub fn delete_playlist(&self, playlist_id: i64) -> Result<()> {
        crate::playlist::delete(&self.conn, playlist_id)
    }

    pub fn rename_playlist(&self, playlist_id: i64, name: &str) -> Result<()> {
        crate::playlist::rename(&self.conn, playlist_id, name)
    }

    pub fn remove_from_playlist(&self, playlist_id: i64, track_ids: &[i64]) -> Result<usize> {
        crate::playlist::remove_tracks(&self.conn, playlist_id, track_ids)
    }

    pub fn move_in_playlist(
        &self,
        playlist_id: i64,
        moving: &[i64],
        before: Option<i64>,
    ) -> Result<()> {
        crate::playlist::move_tracks(&self.conn, playlist_id, moving, before)
    }

    // ---- playlist folders ----

    pub fn playlist_folders(&self) -> Result<Vec<crate::playlist::PlaylistFolder>> {
        crate::playlist::list_folders(&self.conn)
    }

    pub fn create_playlist_folder(&self, name: &str, parent_id: Option<i64>) -> Result<i64> {
        crate::playlist::create_folder(&self.conn, name, parent_id)
    }

    pub fn rename_playlist_folder(&self, folder_id: i64, name: &str) -> Result<()> {
        crate::playlist::rename_folder(&self.conn, folder_id, name)
    }

    pub fn delete_playlist_folder(&self, folder_id: i64) -> Result<()> {
        crate::playlist::delete_folder(&self.conn, folder_id)
    }

    pub fn move_playlist_to_folder(&self, playlist_id: i64, folder_id: Option<i64>) -> Result<()> {
        crate::playlist::move_playlist_to_folder(&self.conn, playlist_id, folder_id)
    }

    // ---- M3U playlists ----

    /// Import an M3U/M3U8 file, indexing missing audio on the way.
    pub fn import_m3u(
        &self,
        path: &Path,
        name: Option<&str>,
        folder_id: Option<i64>,
    ) -> Result<crate::m3u::ImportReport> {
        crate::m3u::import(&self.conn, path, name, folder_id)
    }

    /// Write a playlist as M3U/M3U8. Returns the number of tracks written.
    pub fn export_m3u(&self, playlist_id: i64, path: &Path) -> Result<usize> {
        crate::m3u::export(&self.conn, playlist_id, path)
    }

    /// Write the evaluated smart playlist as M3U/M3U8.
    pub fn export_smart_m3u(&self, playlist_id: i64, path: &Path) -> Result<usize> {
        crate::m3u::export_smart(&self.conn, playlist_id, path)
    }

    // ---- saved searches ----

    pub fn saved_searches(&self) -> Result<Vec<crate::search::SavedSearch>> {
        crate::search::list_saved(&self.conn)
    }

    pub fn create_saved_search(&self, name: &str, query: &str) -> Result<i64> {
        crate::search::create_saved(&self.conn, name, query)
    }

    pub fn update_saved_search(&self, id: i64, name: &str, query: &str) -> Result<()> {
        crate::search::update_saved(&self.conn, id, name, query)
    }

    pub fn delete_saved_search(&self, id: i64) -> Result<()> {
        crate::search::delete_saved(&self.conn, id)
    }

    // ---- smart playlists ----

    pub fn smart_playlists(&self) -> Result<Vec<crate::smart::SmartPlaylist>> {
        crate::smart::list(&self.conn)
    }

    pub fn create_smart_playlist(
        &self,
        name: &str,
        definition: &crate::smart::Definition,
        folder_id: Option<i64>,
    ) -> Result<i64> {
        crate::smart::create(&self.conn, name, definition, folder_id)
    }

    pub fn move_smart_playlist_to_folder(
        &self,
        playlist_id: i64,
        folder_id: Option<i64>,
    ) -> Result<()> {
        crate::smart::set_folder(&self.conn, playlist_id, folder_id)
    }

    pub fn delete_smart_playlist(&self, playlist_id: i64) -> Result<()> {
        crate::smart::delete(&self.conn, playlist_id)
    }

    /// Replace the rule set of an existing smart playlist.
    pub fn update_smart_playlist(
        &self,
        playlist_id: i64,
        definition: &crate::smart::Definition,
    ) -> Result<()> {
        crate::smart::update_definition(&self.conn, playlist_id, definition)
    }

    pub fn rename_smart_playlist(&self, playlist_id: i64, name: &str) -> Result<()> {
        crate::smart::rename(&self.conn, playlist_id, name)
    }

    pub fn smart_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {
        crate::smart::tracks(&self.conn, playlist_id)
    }

    pub fn evaluate_smart(&self, definition: &crate::smart::Definition) -> Result<Vec<Track>> {
        crate::smart::evaluate(&self.conn, definition)
    }

    pub fn refresh_file(&self, path: &Path) -> Result<()> {
        refresh_file(&self.conn, path)
    }

    /// Move files for the accepted organizer rows and return the undo journal.
    pub fn organize_apply(
        &self,
        rows: &[crate::organize::OrganizeRow],
    ) -> Result<crate::organize::ApplyReport> {
        crate::organize::apply(&self.conn, rows)
    }

    /// Write a timestamped database backup into the data directory.
    pub fn create_backup(&self) -> Result<PathBuf> {
        crate::backup::create(&self.conn)
    }

    /// Write a backup to an explicit path.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        crate::backup::create_at(&self.conn, path)
    }

    /// Replace this library with `backup`. Returns the safety backup taken
    /// from the current database before the swap.
    pub fn restore_from(&mut self, backup: &Path) -> Result<PathBuf> {
        crate::backup::validate(backup)?;
        let db_path = self.path.clone().map(Ok).unwrap_or_else(crate::db::path)?;
        let safety = crate::backup::create(&self.conn)?;

        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let placeholder = Connection::open_in_memory()?;
        let old = std::mem::replace(&mut self.conn, placeholder);
        drop(old);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_path.display()));
        }

        if let Err(error) = std::fs::copy(backup, &db_path) {
            // Keep the app usable with the database that is still on disk.
            self.conn = crate::db::open_at(&db_path)?;
            return Err(error).with_context(|| {
                format!(
                    "cannot restore {}; safety backup kept at {}",
                    backup.display(),
                    safety.display()
                )
            });
        }
        self.conn = crate::db::open_at(&db_path)?;
        Ok(safety)
    }

    /// Undo a previous organizer batch.
    pub fn organize_undo(
        &self,
        records: &[crate::organize::MoveRecord],
    ) -> Result<crate::organize::UndoReport> {
        crate::organize::undo(&self.conn, records)
    }

    /// Index loose files and folders, e.g. from an OS drag and drop. Returns
    /// the audio files that were indexed.
    pub fn import_paths(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        import_paths(&self.conn, paths)
    }

    pub fn db_path(&self) -> Result<PathBuf> {
        self.path.clone().map(Ok).unwrap_or_else(crate::db::path)
    }
}

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "m4a", "m4b", "aac", "wav", "wv", "ape", "wma", "aiff", "aif",
    "mpc", "tak", "dsf",
];

/// Is this path a file type the library can index?
pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Index loose files and folders, returning the audio files that were indexed.
///
/// Used by the GUI when files are dropped from the OS file manager.
pub fn import_paths(conn: &Connection, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut indexed = std::collections::BTreeSet::new();
    for path in paths {
        if path.is_dir() {
            scan(conn, path, false)?;
            for entry in WalkDir::new(path).follow_links(true).into_iter().flatten() {
                if entry.file_type().is_file() && is_audio_path(entry.path()) {
                    indexed.insert(entry.path().to_path_buf());
                }
            }
        } else if path.is_file() && is_audio_path(path) {
            refresh_file(conn, path)?;
            indexed.insert(path.clone());
        }
    }
    Ok(indexed.into_iter().collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<i32>,
    pub track_no: Option<i32>,
    pub disc_no: Option<i32>,
    pub duration_secs: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub format: String,
    pub rating: u8,
    pub play_count: u32,
    pub skip_count: u32,
    pub last_played: Option<i64>,
    pub date_added: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumGroup {
    pub album: String,
    pub artist: String,
    pub track_count: usize,
    /// Lowest track id, used to load the album's cover art.
    pub representative_id: i64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub tracks: u64,
    pub artists: u64,
    pub albums: u64,
    pub total_secs: u64,
    pub total_bytes: u64,
}

/// Browsing filter. Every field is ANDed; `text` is parsed as an advanced
/// search (plain text plus `artist:`, `year:` and similar filters).
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub genre: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub text: String,
    /// Only tracks that have never been played.
    pub never_played: bool,
    /// Only tracks whose last play is older than this many days, or never.
    pub not_played_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Duration,
    Added,
    Rating,
    PlayCount,
    LastPlayed,
}

impl SortKey {
    fn column(self) -> &'static str {
        match self {
            SortKey::Title => "title",
            SortKey::Artist => "artist",
            SortKey::Album => "album",
            SortKey::Genre => "genre",
            SortKey::Year => "year",
            SortKey::Duration => "duration_secs",
            SortKey::Added => "date_added",
            SortKey::Rating => "rating",
            SortKey::PlayCount => "play_count",
            SortKey::LastPlayed => "last_played",
        }
    }
}

pub(crate) const TRACK_COLUMNS: &str =
    "id, path, title, artist, album, album_artist, genre, year, track_no,
     disc_no, duration_secs, sample_rate, bitrate, format,
     rating, play_count, skip_count, last_played, date_added";

pub(crate) fn row_to_track(r: &rusqlite::Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        album_artist: r.get(5)?,
        genre: r.get(6)?,
        year: r.get(7)?,
        track_no: r.get(8)?,
        disc_no: r.get(9)?,
        duration_secs: r.get(10)?,
        sample_rate: r.get(11)?,
        bitrate: r.get(12)?,
        format: r.get(13)?,
        rating: r.get(14)?,
        play_count: r.get(15)?,
        skip_count: r.get(16)?,
        last_played: r.get(17)?,
        date_added: r.get(18)?,
    })
}

pub fn query(conn: &Connection, filter: &Filter, sort: SortKey, asc: bool) -> Result<Vec<Track>> {
    let mut sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE 1=1");
    let mut args: Vec<Value> = Vec::new();
    let search = crate::search::parse(&filter.text);

    if let Some(g) = &filter.genre {
        sql.push_str(" AND genre = ?");
        args.push(Value::Text(g.clone()));
    }
    if let Some(a) = &filter.artist {
        sql.push_str(" AND artist = ?");
        args.push(Value::Text(a.clone()));
    }
    if let Some(al) = &filter.album {
        sql.push_str(" AND album = ?");
        args.push(Value::Text(al.clone()));
    }
    if !search.text.trim().is_empty() {
        // Prefilter through the trigram FTS index, then keep the LIKE check so
        // results match the old substring search exactly. Short queries fall
        // back to the plain scan because a trigram needs three characters.
        if let Some(match_expr) = fts_phrase(&search.text) {
            sql.push_str(" AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)");
            args.push(Value::Text(match_expr));
        }
        sql.push_str(
            " AND (title LIKE ? ESCAPE '\\' OR artist LIKE ? ESCAPE '\\'
                   OR album LIKE ? ESCAPE '\\' OR genre LIKE ? ESCAPE '\\')",
        );
        let like = like_pattern(search.text.trim());
        for _ in 0..4 {
            args.push(Value::Text(like.clone()));
        }
    }
    for condition in search.conditions {
        match condition {
            crate::search::Condition::Text {
                field,
                value,
                exact,
            } => {
                if exact {
                    sql.push_str(&format!(" AND {} = ? COLLATE NOCASE", field.column()));
                    args.push(Value::Text(value));
                } else {
                    if let Some(match_expr) = fts_phrase(&value) {
                        sql.push_str(
                            " AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)",
                        );
                        args.push(Value::Text(format!("{} : {match_expr}", field.column())));
                    }
                    sql.push_str(&format!(" AND {} LIKE ? ESCAPE '\\'", field.column()));
                    args.push(Value::Text(like_pattern(&value)));
                }
            }
            crate::search::Condition::Number { field, op, value } => {
                sql.push_str(&format!(" AND {} {} ?", field.column(), op.sql()));
                args.push(Value::Integer(value));
            }
            crate::search::Condition::Range { field, from, to } => {
                sql.push_str(&format!(" AND {} BETWEEN ? AND ?", field.column()));
                args.push(Value::Integer(from));
                args.push(Value::Integer(to));
            }
        }
    }

    if filter.never_played {
        sql.push_str(" AND play_count = 0");
    }
    if let Some(days) = filter.not_played_days {
        sql.push_str(" AND (last_played IS NULL OR last_played < unixepoch() - ?)");
        args.push(Value::Integer(i64::from(days) * 86_400));
    }

    sql.push_str(&format!(
        " ORDER BY {} {}, artist COLLATE NOCASE, album COLLATE NOCASE, disc_no, track_no",
        sort.column(),
        if asc { "ASC" } else { "DESC" }
    ));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), row_to_track)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Escape LIKE wildcards so plain search text is matched literally.
fn like_pattern(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// FTS5 trigram phrase for a raw search string.
///
/// Returns `None` for strings shorter than one trigram: the trigram tokenizer
/// cannot match them, so the caller keeps the plain LIKE scan.
fn fts_phrase(text: &str) -> Option<String> {
    let text = text.trim();
    if text.chars().count() < 3 {
        return None;
    }
    Some(format!("\"{}\"", text.replace('"', "\"\"")))
}

/// Tracks for a set of ids, in the order requested.
pub fn tracks_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<Track>> {
    let mut out = Vec::new();
    for id in ids {
        let mut stmt =
            conn.prepare(&format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1"))?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            out.push(row_to_track(row)?);
        }
    }
    Ok(out)
}

pub fn list(conn: &Connection, text: Option<&str>) -> Result<Vec<Track>> {
    let filter = Filter {
        text: text.unwrap_or("").to_string(),
        ..Default::default()
    };
    query(conn, &filter, SortKey::Artist, true)
}

/// Tracks whose files cannot be found at their indexed path.
pub fn missing_tracks(conn: &Connection) -> Result<Vec<Track>> {
    Ok(list(conn, None)?
        .into_iter()
        .filter(|track| !Path::new(&track.path).is_file())
        .collect())
}

/// Delete library rows without touching the files. Returns the number removed.
pub fn remove_tracks(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("DELETE FROM tracks WHERE id IN ({placeholders})");
    Ok(conn.execute(&sql, params_from_iter(ids.iter()))?)
}

/// Set a 0..5 star rating. Values outside the range are clamped.
pub fn set_rating(conn: &Connection, track_id: i64, rating: u8) -> Result<()> {
    let rating = rating.min(5) as i64;
    conn.execute(
        "UPDATE tracks SET rating = ?1 WHERE id = ?2",
        params![rating, track_id],
    )?;
    Ok(())
}

/// Count a play and update the last-played timestamp.
pub fn record_play(conn: &Connection, track_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE tracks
         SET play_count = play_count + 1, last_played = unixepoch()
         WHERE id = ?1",
        params![track_id],
    )?;
    Ok(())
}

pub fn record_skip(conn: &Connection, track_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE tracks SET skip_count = skip_count + 1 WHERE id = ?1",
        params![track_id],
    )?;
    Ok(())
}

/// Groups of tracks that look like the same recording: same artist, title and
/// album after trimming, collapsing whitespace and ignoring case. Empty titles
/// are skipped because they cannot be identified reliably.
pub fn duplicate_groups(conn: &Connection) -> Result<Vec<Vec<Track>>> {
    Ok(find_duplicate_groups(list(conn, None)?))
}

pub(crate) fn normalize_tag(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn album_groups(conn: &Connection) -> Result<Vec<AlbumGroup>> {
    // Only the five fields the grid needs. Loading full Track rows (and sorting
    // them in SQL) made opening the grid slow on large libraries.
    let mut stmt = conn.prepare(
        "SELECT album, artist, album_artist, id, path
         FROM tracks
         WHERE TRIM(album) <> ''
         ORDER BY LOWER(album), LOWER(artist)",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut groups: BTreeMap<(String, String), AlbumGroup> = BTreeMap::new();
    for row in rows {
        let (album, artist, album_artist, id, path) = row?;
        let artist = if album_artist.trim().is_empty() {
            artist
        } else {
            album_artist
        };
        let key = (album.to_lowercase(), artist.to_lowercase());
        if let Some(entry) = groups.get_mut(&key) {
            entry.track_count += 1;
            if id < entry.representative_id {
                entry.representative_id = id;
                entry.path = path;
            }
        } else {
            groups.insert(
                key,
                AlbumGroup {
                    album,
                    artist,
                    track_count: 1,
                    representative_id: id,
                    path,
                },
            );
        }
    }

    // The map key is the same lowercase pair used for sorting.
    Ok(groups.into_values().collect())
}

fn album_artist_of(track: &Track) -> String {
    if track.album_artist.trim().is_empty() {
        track.artist.clone()
    } else {
        track.album_artist.clone()
    }
}

pub fn album_tracks(conn: &Connection, album: &str, artist: &str) -> Result<Vec<Track>> {
    let album = album.to_lowercase();
    let artist = artist.to_lowercase();
    Ok(list(conn, None)?
        .into_iter()
        .filter(|track| {
            track.album.to_lowercase() == album && album_artist_of(track).to_lowercase() == artist
        })
        .collect())
}

/// Common sidecar cover names, in preference order.
const SIDECAR_NAMES: [&str; 9] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
];

/// A `cover.jpg`/`folder.png` image sitting next to the audio file.
pub fn sidecar_cover(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    for name in SIDECAR_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Match names case-insensitively for libraries coming from other OSes.
    let wanted: Vec<String> = SIDECAR_NAMES.iter().map(|name| name.to_string()).collect();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if wanted.contains(&name) {
            return Some(path);
        }
    }
    None
}

/// Embedded front cover, falling back to a sidecar image next to the file.
pub fn cover_art(path: &Path) -> Option<Vec<u8>> {
    let embedded = Probe::open(path)
        .ok()
        .and_then(|probe| probe.read().ok())
        .and_then(|tagged| {
            let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
            Some(tag.pictures().first()?.data().to_vec())
        });
    embedded.or_else(|| sidecar_cover(path).and_then(|cover| std::fs::read(cover).ok()))
}

fn mime_for(value: &str) -> Option<MimeType> {
    match value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" | "image/jpg" => Some(MimeType::Jpeg),
        "image/png" => Some(MimeType::Png),
        "image/gif" => Some(MimeType::Gif),
        "image/bmp" => Some(MimeType::Bmp),
        "image/tiff" => Some(MimeType::Tiff),
        "image/webp" => Some(MimeType::Unknown("image/webp".to_string())),
        _ => None,
    }
}

/// Replace the front cover embedded in a track. Other pictures are preserved.
pub fn embed_cover(path: &Path, data: &[u8], mime: &str) -> Result<()> {
    let mut tagged = Probe::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read()?;
    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .context("file has no writable tag")?;
    let picture =
        Picture::new_unchecked(PictureType::CoverFront, mime_for(mime), None, data.to_vec());
    if tag.pictures().is_empty() {
        tag.push_picture(picture);
    } else {
        tag.set_picture(0, picture);
    }
    tag.save_to_path(path, WriteOptions::default())
        .with_context(|| format!("cannot write cover to {}", path.display()))?;
    Ok(())
}

/// Write JPEG bytes as `cover.jpg` beside the audio file.
pub fn save_cover_sidecar(path: &Path, jpeg: &[u8]) -> Result<PathBuf> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let target = dir.join("cover.jpg");
    let tmp = dir.join("cover.jpg.tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, &target)?;
    Ok(target)
}

pub fn find_duplicate_groups(tracks: Vec<Track>) -> Vec<Vec<Track>> {
    use std::collections::BTreeMap;

    let mut groups: BTreeMap<(String, String, String), Vec<Track>> = BTreeMap::new();
    for track in tracks {
        let title = normalize_tag(&track.title);
        if title.is_empty() {
            continue;
        }
        let key = (
            normalize_tag(&track.artist),
            title,
            normalize_tag(&track.album),
        );
        groups.entry(key).or_default().push(track);
    }

    let mut out: Vec<Vec<Track>> = groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect();
    for group in &mut out {
        group.sort_by(|a, b| a.path.cmp(&b.path));
    }
    out.sort_by(|a, b| {
        let left = (&a[0].artist, &a[0].title, &a[0].album);
        let right = (&b[0].artist, &b[0].title, &b[0].album);
        left.cmp(&right)
    });
    out
}

fn distinct(
    conn: &Connection,
    column: &str,
    conditions: &[&str],
    args: &[&str],
) -> Result<Vec<String>> {
    // `column` and `conditions` are internal literals, never user input.
    let mut sql = format!(
        "SELECT DISTINCT {column} FROM tracks
         WHERE {column} IS NOT NULL AND TRIM({column}) <> ''"
    );
    for c in conditions {
        sql.push_str(" AND ");
        sql.push_str(c);
    }
    sql.push_str(&format!(" ORDER BY {column} COLLATE NOCASE"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn genres(conn: &Connection) -> Result<Vec<String>> {
    distinct(conn, "genre", &[], &[])
}

pub fn artists(conn: &Connection, genre: Option<&str>) -> Result<Vec<String>> {
    match genre {
        Some(g) => distinct(conn, "artist", &["genre = ?"], &[g]),
        None => distinct(conn, "artist", &[], &[]),
    }
}

pub fn albums(conn: &Connection, artist: Option<&str>, genre: Option<&str>) -> Result<Vec<String>> {
    match (artist, genre) {
        (Some(a), Some(g)) => distinct(conn, "album", &["artist = ?", "genre = ?"], &[a, g]),
        (Some(a), None) => distinct(conn, "album", &["artist = ?"], &[a]),
        (None, Some(g)) => distinct(conn, "album", &["genre = ?"], &[g]),
        (None, None) => distinct(conn, "album", &[], &[]),
    }
}

pub fn path_for_id(conn: &Connection, id: i64) -> Result<Option<PathBuf>> {
    let mut stmt = conn.prepare("SELECT path FROM tracks WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(row) => Some(PathBuf::from(row.get::<_, String>(0)?)),
        None => None,
    })
}

pub fn stats(conn: &Connection) -> Result<Stats> {
    let (tracks, total_secs, total_bytes): (u64, u64, u64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(duration_secs),0), COALESCE(SUM(size_bytes),0) FROM tracks",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let artists: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT artist) FROM tracks WHERE artist <> ''",
        [],
        |r| r.get(0),
    )?;
    let albums: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT artist || char(1) || album) FROM tracks WHERE album <> ''",
        [],
        |r| r.get(0),
    )?;
    Ok(Stats {
        tracks,
        artists,
        albums,
        total_secs,
        total_bytes,
    })
}

/// Live counters from a scan. The worker throttles how often these are sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanProgress {
    pub path: PathBuf,
    /// Audio files discovered before the scan started.
    pub total: usize,
    pub found: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub failed: usize,
}

/// Outcome of a full or cancelled scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Audio files discovered before the scan started.
    pub total: usize,
    pub found: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub cancelled: bool,
}

/// Walk `root`, read tags with lofty, upsert into SQLite.
///
/// This is the incremental scan: unchanged paths are identified by mtime and
/// size, and writes are committed in batches so a huge first import remains
/// visible and interruptible.
pub fn scan(conn: &Connection, root: &Path, rescan: bool) -> Result<usize> {
    Ok(scan_with_progress(conn, root, rescan, |_| true)?.found)
}

/// Incremental, cancellable scan with progress callbacks.
///
/// The callback first receives a snapshot with the pre-counted total and
/// `found = 0`, then runs once per audio file after that file has been handled.
/// Returning `false` stops the walk; the batch being written is committed and
/// the report has `cancelled = true`.
pub fn scan_with_progress<F>(
    conn: &Connection,
    root: &Path,
    rescan: bool,
    mut progress: F,
) -> Result<ScanReport>
where
    F: FnMut(ScanProgress) -> bool,
{
    anyhow::ensure!(root.is_dir(), "not a directory: {}", root.display());

    let existing = if rescan {
        None
    } else {
        Some(existing_files(conn, root)?)
    };
    let mut report = ScanReport {
        total: count_audio_files(root),
        ..Default::default()
    };
    // Report the total before reading any tags so the UI can show a bar.
    if !progress(progress_snapshot(&report, root)) {
        report.cancelled = true;
        return Ok(report);
    }

    let mut stmt = conn.prepare_cached(
        r#"INSERT INTO tracks
             (path, title, artist, album, album_artist, genre, year, track_no, disc_no,
              duration_secs, sample_rate, bitrate, channels, format, size_bytes, mtime,
              date_added)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,unixepoch())
           ON CONFLICT(path) DO UPDATE SET
             title=excluded.title, artist=excluded.artist, album=excluded.album,
             album_artist=excluded.album_artist, genre=excluded.genre, year=excluded.year,
             track_no=excluded.track_no, disc_no=excluded.disc_no,
             duration_secs=excluded.duration_secs, sample_rate=excluded.sample_rate,
             bitrate=excluded.bitrate, channels=excluded.channels, format=excluded.format,
             size_bytes=excluded.size_bytes, mtime=excluded.mtime"#,
    )?;

    // Batch commits keep the WAL small and let the UI see rows appearing during
    // a first import instead of waiting for the whole library.
    const BATCH: usize = 500;
    let mut tx = conn.unchecked_transaction()?;
    let mut pending = 0usize;

    for entry in WalkDir::new(root).follow_links(true) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_ascii_lowercase(),
            None => continue,
        };
        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(error) => {
                tracing::warn!("skip {}: {error}", path.display());
                report.found += 1;
                report.failed += 1;
                if !progress(progress_snapshot(&report, path)) {
                    report.cancelled = true;
                    break;
                }
                continue;
            }
        };
        let mtime = file_mtime(&meta);

        report.found += 1;

        if let Some(existing) = &existing {
            let same = existing
                .get(&path.to_string_lossy().to_string())
                .is_some_and(|(old_mtime, old_size)| {
                    *old_mtime == mtime && *old_size == meta.len() as i64
                });
            if same {
                report.unchanged += 1;
                if !progress(progress_snapshot(&report, path)) {
                    report.cancelled = true;
                    break;
                }
                continue;
            }
        }

        let info = match read_tags(path) {
            Ok(info) => info,
            Err(error) => {
                tracing::warn!("skip {}: {error}", path.display());
                report.failed += 1;
                if !progress(progress_snapshot(&report, path)) {
                    report.cancelled = true;
                    break;
                }
                continue;
            }
        };

        stmt.execute(params![
            path.to_string_lossy(),
            info.title,
            info.artist,
            info.album,
            info.album_artist,
            info.genre,
            info.year,
            info.track_no,
            info.disc_no,
            info.duration_secs,
            info.sample_rate,
            info.bitrate,
            info.channels,
            info.format,
            meta.len() as i64,
            mtime,
        ])?;
        report.indexed += 1;
        pending += 1;

        if pending >= BATCH {
            tx.commit()?;
            tx = conn.unchecked_transaction()?;
            pending = 0;
        }

        if !progress(progress_snapshot(&report, path)) {
            report.cancelled = true;
            break;
        }
    }

    tx.commit()?;
    Ok(report)
}

fn progress_snapshot(report: &ScanReport, path: &Path) -> ScanProgress {
    ScanProgress {
        path: path.to_path_buf(),
        total: report.total,
        found: report.found,
        indexed: report.indexed,
        unchanged: report.unchanged,
        failed: report.failed,
    }
}

/// Count audio files before indexing, so progress has a denominator. This walk
/// only checks names and file types; tags are read in the real pass.
fn count_audio_files(root: &Path) -> usize {
    WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file() && is_audio_path(entry.path()))
        .count()
}

/// Existing path -> (mtime, size) rows under one root, loaded once per scan.
fn existing_files(conn: &Connection, root: &Path) -> Result<HashMap<String, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT path, mtime, size_bytes FROM tracks")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut existing = HashMap::new();
    for row in rows {
        let (path, mtime, size) = row?;
        if Path::new(&path).starts_with(root) {
            existing.insert(path, (mtime, size));
        }
    }
    Ok(existing)
}

pub(crate) fn file_mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Re-read one file and upsert it (used after tag edits).
pub fn refresh_file(conn: &Connection, path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let mtime = file_mtime(&meta);
    let info = read_tags(path)?;

    conn.execute(
        r#"INSERT INTO tracks
             (path, title, artist, album, album_artist, genre, year, track_no, disc_no,
              duration_secs, sample_rate, bitrate, channels, format, size_bytes, mtime,
              date_added)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,unixepoch())
           ON CONFLICT(path) DO UPDATE SET
             title=excluded.title, artist=excluded.artist, album=excluded.album,
             album_artist=excluded.album_artist, genre=excluded.genre, year=excluded.year,
             track_no=excluded.track_no, disc_no=excluded.disc_no,
             duration_secs=excluded.duration_secs, sample_rate=excluded.sample_rate,
             bitrate=excluded.bitrate, channels=excluded.channels, format=excluded.format,
             size_bytes=excluded.size_bytes, mtime=excluded.mtime"#,
        params![
            path.to_string_lossy(),
            info.title,
            info.artist,
            info.album,
            info.album_artist,
            info.genre,
            info.year,
            info.track_no,
            info.disc_no,
            info.duration_secs,
            info.sample_rate,
            info.bitrate,
            info.channels,
            info.format,
            meta.len() as i64,
            mtime,
        ],
    )?;
    Ok(())
}

struct TagInfo {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    year: Option<i32>,
    track_no: Option<i32>,
    disc_no: Option<i32>,
    duration_secs: u64,
    sample_rate: u32,
    bitrate: u32,
    channels: u8,
    format: String,
}

/// The tag subset reconciliation needs, so the full `TagInfo` stays private.
pub(crate) struct AudioSummary {
    pub duration_secs: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub format: String,
}

pub(crate) fn audio_summary(path: &Path) -> Option<AudioSummary> {
    let info = read_tags(path).ok()?;
    Some(AudioSummary {
        duration_secs: info.duration_secs,
        title: info.title,
        artist: info.artist,
        album: info.album,
        format: info.format,
    })
}

fn read_tags(path: &Path) -> Result<TagInfo> {
    let tagged = Probe::open(path)?.read()?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut info = TagInfo {
        title: stem,
        artist: String::from("Unknown Artist"),
        album: String::from("Unknown Album"),
        album_artist: String::new(),
        genre: String::new(),
        year: None,
        track_no: None,
        disc_no: None,
        duration_secs: props.duration().as_secs(),
        sample_rate: props.sample_rate().unwrap_or(0),
        bitrate: props.audio_bitrate().unwrap_or(0),
        channels: props.channels().unwrap_or(0),
        format: format!("{:?}", tagged.file_type()).to_ascii_uppercase(),
    };

    if let Some(tag) = tag {
        if let Some(v) = tag.title().filter(|s| !s.trim().is_empty()) {
            info.title = v.into_owned();
        }
        if let Some(v) = tag.artist().filter(|s| !s.trim().is_empty()) {
            info.artist = v.into_owned();
        }
        if let Some(v) = tag.album().filter(|s| !s.trim().is_empty()) {
            info.album = v.into_owned();
        }
        if let Some(v) = tag.get_string(&ItemKey::AlbumArtist) {
            info.album_artist = v.to_string();
        }
        if let Some(v) = tag.genre().filter(|s| !s.trim().is_empty()) {
            info.genre = v.into_owned();
        }
        info.year = tag.year().map(|y| y as i32);
        info.track_no = tag.track().map(|t| t as i32);
        info.disc_no = tag.disk().map(|d| d as i32);
    }

    Ok(info)
}
