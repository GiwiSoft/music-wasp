use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
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
        "PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;
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
            added_at      INTEGER NOT NULL DEFAULT (unixepoch()),
            rating        INTEGER NOT NULL DEFAULT 0,
            play_count    INTEGER NOT NULL DEFAULT 0,
            skip_count    INTEGER NOT NULL DEFAULT 0,
            last_played   INTEGER,
            date_added    INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
        CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
        CREATE INDEX IF NOT EXISTS idx_tracks_genre ON tracks(genre);

        -- Optional hierarchy for grouping playlists. Folders can be nested;
        -- deleting a folder moves its children up one level.
        CREATE TABLE IF NOT EXISTS playlist_folders (
            id        INTEGER PRIMARY KEY,
            name      TEXT NOT NULL,
            parent_id INTEGER REFERENCES playlist_folders(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS playlists (
            id        INTEGER PRIMARY KEY,
            name      TEXT NOT NULL,
            folder_id INTEGER REFERENCES playlist_folders(id) ON DELETE SET NULL
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
            definition TEXT NOT NULL,
            folder_id  INTEGER REFERENCES playlist_folders(id) ON DELETE SET NULL
        );

        -- Named queries shown in the search box menu.
        CREATE TABLE IF NOT EXISTS saved_searches (
            id    INTEGER PRIMARY KEY,
            name  TEXT NOT NULL,
            query TEXT NOT NULL
        );

        -- Internet radio stations; playback streams straight from the URL.
        CREATE TABLE IF NOT EXISTS radio_stations (
            id       INTEGER PRIMARY KEY,
            name     TEXT NOT NULL,
            url      TEXT NOT NULL UNIQUE,
            genre    TEXT NOT NULL DEFAULT '',
            added_at INTEGER NOT NULL DEFAULT (unixepoch())
        );

        -- Podcast subscriptions and their episodes. Progress is kept per
        -- episode so the UI can show listened state.
        CREATE TABLE IF NOT EXISTS podcast_feeds (
            id           INTEGER PRIMARY KEY,
            url          TEXT NOT NULL UNIQUE,
            title        TEXT NOT NULL DEFAULT '',
            description  TEXT NOT NULL DEFAULT '',
            image_url    TEXT NOT NULL DEFAULT '',
            last_checked INTEGER,
            added_at     INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS podcast_episodes (
            id            INTEGER PRIMARY KEY,
            feed_id       INTEGER NOT NULL REFERENCES podcast_feeds(id) ON DELETE CASCADE,
            guid          TEXT NOT NULL,
            title         TEXT NOT NULL DEFAULT '',
            description   TEXT NOT NULL DEFAULT '',
            audio_url     TEXT NOT NULL,
            image_url     TEXT NOT NULL DEFAULT '',
            published     TEXT NOT NULL DEFAULT '',
            published_at  INTEGER NOT NULL DEFAULT 0,
            duration_secs INTEGER NOT NULL DEFAULT 0,
            played        INTEGER NOT NULL DEFAULT 0,
            position_secs INTEGER NOT NULL DEFAULT 0,
            UNIQUE (feed_id, guid)
        );
        CREATE INDEX IF NOT EXISTS idx_podcast_episodes_feed
            ON podcast_episodes(feed_id, published_at DESC);
        "#,
    )?;

    // Databases created before playlist folders existed need the column added
    // in place. `CREATE TABLE IF NOT EXISTS` above cannot change them.
    if !has_column(conn, "playlists", "folder_id")? {
        conn.execute(
            "ALTER TABLE playlists ADD COLUMN folder_id INTEGER
             REFERENCES playlist_folders(id) ON DELETE SET NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_playlists_folder ON playlists(folder_id)",
        [],
    )?;
    // Smart playlists learned folders after the first schema.
    if !has_column(conn, "smart_playlists", "folder_id")? {
        conn.execute(
            "ALTER TABLE smart_playlists ADD COLUMN folder_id INTEGER
             REFERENCES playlist_folders(id) ON DELETE SET NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_smart_playlists_folder ON smart_playlists(folder_id)",
        [],
    )?;

    // Playback statistics were added after the first schema. Keep these
    // additive migrations so old libraries keep their rows and tags.
    for (column, ddl) in [
        (
            "rating",
            "ALTER TABLE tracks ADD COLUMN rating INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "play_count",
            "ALTER TABLE tracks ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "skip_count",
            "ALTER TABLE tracks ADD COLUMN skip_count INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "last_played",
            "ALTER TABLE tracks ADD COLUMN last_played INTEGER",
        ),
        (
            "date_added",
            "ALTER TABLE tracks ADD COLUMN date_added INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !has_column(conn, "tracks", column)? {
            conn.execute(ddl, [])?;
        }
    }
    conn.execute(
        "UPDATE tracks SET date_added = added_at WHERE date_added = 0",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tracks_rating ON tracks(rating)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tracks_play_count ON tracks(play_count)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tracks_last_played ON tracks(last_played)",
        [],
    )?;

    // Full-text index over the searchable tag columns. The trigram tokenizer
    // keeps the substring semantics of the previous LIKE scans while letting
    // SQLite answer them from an index. External content avoids duplicating
    // the rows; the triggers keep the index in sync for every write path,
    // including the scan upserts and tag refreshes.
    let has_fts: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'tracks_fts'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
            title, artist, album, album_artist, genre, format,
            content = 'tracks',
            content_rowid = 'id',
            tokenize = 'trigram'
        );
        CREATE TRIGGER IF NOT EXISTS tracks_fts_ai AFTER INSERT ON tracks BEGIN
            INSERT INTO tracks_fts(rowid, title, artist, album, album_artist, genre, format)
            VALUES (new.id, new.title, new.artist, new.album, new.album_artist,
                    new.genre, new.format);
        END;
        CREATE TRIGGER IF NOT EXISTS tracks_fts_ad AFTER DELETE ON tracks BEGIN
            INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, album_artist,
                                   genre, format)
            VALUES ('delete', old.id, old.title, old.artist, old.album, old.album_artist,
                    old.genre, old.format);
        END;
        CREATE TRIGGER IF NOT EXISTS tracks_fts_au
        AFTER UPDATE OF title, artist, album, album_artist, genre, format ON tracks BEGIN
            INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, album_artist,
                                   genre, format)
            VALUES ('delete', old.id, old.title, old.artist, old.album, old.album_artist,
                    old.genre, old.format);
            INSERT INTO tracks_fts(rowid, title, artist, album, album_artist, genre, format)
            VALUES (new.id, new.title, new.artist, new.album, new.album_artist,
                    new.genre, new.format);
        END;
        "#,
    )?;
    // Existing libraries predate the index: fill it once on upgrade.
    if !has_fts {
        conn.execute("INSERT INTO tracks_fts(tracks_fts) VALUES ('rebuild')", [])?;
    }
    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
