use anyhow::Result;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, AudioFile, ItemKey};
use lofty::probe::Probe;
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
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

    /// Groups of tracks sharing normalized artist, title and album.
    pub fn duplicate_groups(&self) -> Result<Vec<Vec<Track>>> {
        duplicate_groups(&self.conn)
    }

    /// Delete library rows only. Audio files stay on disk, and playlist entries
    /// disappear through the `ON DELETE CASCADE` foreign keys.
    pub fn remove_tracks(&self, ids: &[i64]) -> Result<usize> {
        remove_tracks(&self.conn, ids)
    }

    pub fn scan(&self, root: &Path, rescan: bool) -> Result<usize> {
        scan(&self.conn, root, rescan)
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

    // ---- smart playlists ----

    pub fn smart_playlists(&self) -> Result<Vec<crate::smart::SmartPlaylist>> {
        crate::smart::list(&self.conn)
    }

    pub fn create_smart_playlist(
        &self,
        name: &str,
        definition: &crate::smart::Definition,
    ) -> Result<i64> {
        crate::smart::create(&self.conn, name, definition)
    }

    pub fn delete_smart_playlist(&self, playlist_id: i64) -> Result<()> {
        crate::smart::delete(&self.conn, playlist_id)
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub tracks: u64,
    pub artists: u64,
    pub albums: u64,
    pub total_secs: u64,
    pub total_bytes: u64,
}

/// Browsing filter. Every field is ANDed; `text` matches title/artist/album/genre.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub genre: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub text: String,
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
            SortKey::Added => "added_at",
        }
    }
}

const TRACK_COLUMNS: &str = "id, path, title, artist, album, album_artist, genre, year, track_no,
     disc_no, duration_secs, sample_rate, bitrate, format";

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
    })
}

pub fn query(conn: &Connection, filter: &Filter, sort: SortKey, asc: bool) -> Result<Vec<Track>> {
    let mut sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE 1=1");
    let mut args: Vec<String> = Vec::new();

    if let Some(g) = &filter.genre {
        sql.push_str(" AND genre = ?");
        args.push(g.clone());
    }
    if let Some(a) = &filter.artist {
        sql.push_str(" AND artist = ?");
        args.push(a.clone());
    }
    if let Some(al) = &filter.album {
        sql.push_str(" AND album = ?");
        args.push(al.clone());
    }
    if !filter.text.trim().is_empty() {
        sql.push_str(" AND (title LIKE ? OR artist LIKE ? OR album LIKE ? OR genre LIKE ?)");
        let like = format!("%{}%", filter.text.trim());
        for _ in 0..4 {
            args.push(like.clone());
        }
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

/// Groups of tracks that look like the same recording: same artist, title and
/// album after trimming, collapsing whitespace and ignoring case. Empty titles
/// are skipped because they cannot be identified reliably.
pub fn duplicate_groups(conn: &Connection) -> Result<Vec<Vec<Track>>> {
    Ok(find_duplicate_groups(list(conn, None)?))
}

fn normalize_tag(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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

/// Embedded front-cover picture bytes for a track, if any.
pub fn cover_art(path: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().first()?;
    Some(pic.data().to_vec())
}

/// Walk `root`, read tags with lofty, upsert into SQLite.
pub fn scan(conn: &Connection, root: &Path, rescan: bool) -> Result<usize> {
    anyhow::ensure!(root.is_dir(), "not a directory: {}", root.display());

    let mut seen = 0usize;

    let mut stmt = conn.prepare_cached(
        r#"INSERT INTO tracks
             (path, title, artist, album, album_artist, genre, year, track_no, disc_no,
              duration_secs, sample_rate, bitrate, channels, format, size_bytes, mtime)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
           ON CONFLICT(path) DO UPDATE SET
             title=excluded.title, artist=excluded.artist, album=excluded.album,
             album_artist=excluded.album_artist, genre=excluded.genre, year=excluded.year,
             track_no=excluded.track_no, disc_no=excluded.disc_no,
             duration_secs=excluded.duration_secs, sample_rate=excluded.sample_rate,
             bitrate=excluded.bitrate, channels=excluded.channels, format=excluded.format,
             size_bytes=excluded.size_bytes, mtime=excluded.mtime"#,
    )?;

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
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if !rescan {
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT mtime FROM tracks WHERE path = ?1",
                    params![path.to_string_lossy()],
                    |r| r.get(0),
                )
                .ok();
            if existing == Some(mtime) {
                seen += 1;
                continue;
            }
        }

        let info = match read_tags(path) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("skip {}: {e}", path.display());
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
        seen += 1;
    }

    Ok(seen)
}

/// Re-read one file and upsert it (used after tag edits).
pub fn refresh_file(conn: &Connection, path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let info = read_tags(path)?;

    conn.execute(
        r#"INSERT INTO tracks
             (path, title, artist, album, album_artist, genre, year, track_no, disc_no,
              duration_secs, sample_rate, bitrate, channels, format, size_bytes, mtime)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
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
