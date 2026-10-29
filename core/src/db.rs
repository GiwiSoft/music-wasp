use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Directory holding the library database and user settings.
pub fn data_dir() -> Result<PathBuf> {
    // MUSICWASP_DATA_DIR enables portable / separate libraries.
    let dir = match std::env::var_os("MUSICWASP_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => dirs::data_dir().context("no data dir")?.join("musicwasp"),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn path() -> Result<PathBuf> {
    Ok(data_dir()?.join("library.db"))
}

pub fn open() -> Result<Connection> {
    open_at(&path()?)
}

pub fn open_at(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tracks (
            id            INTEGER PRIMARY KEY,
            path          TEXT NOT NULL UNIQUE,
            title         TEXT NOT NULL DEFAULT '',
            artist        TEXT NOT NULL DEFAULT '',
            album         TEXT NOT NULL DEFAULT '',
            album_artist  TEXT NOT NULL DEFAULT '',
            genre         TEXT NOT NULL DEFAULT '',
            year          INTEGER,
            track_no      INTEGER,
            disc_no       INTEGER,
            duration_secs INTEGER NOT NULL DEFAULT 0,
            sample_rate   INTEGER NOT NULL DEFAULT 0,
            bitrate       INTEGER NOT NULL DEFAULT 0,
            channels      INTEGER NOT NULL DEFAULT 0,
            format        TEXT NOT NULL DEFAULT '',
            size_bytes    INTEGER NOT NULL DEFAULT 0,
            mtime         INTEGER NOT NULL DEFAULT 0,
            added_at      INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
        CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
        CREATE INDEX IF NOT EXISTS idx_tracks_genre ON tracks(genre);

        CREATE TABLE IF NOT EXISTS playlists (
            id   INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playlist_tracks (
            playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            position    INTEGER NOT NULL,
            PRIMARY KEY (playlist_id, track_id)
        );
        CREATE INDEX IF NOT EXISTS idx_pt_playlist ON playlist_tracks(playlist_id, position);

        -- Rule-based playlists; `definition` holds the JSON rule set.
        CREATE TABLE IF NOT EXISTS smart_playlists (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            definition TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}
