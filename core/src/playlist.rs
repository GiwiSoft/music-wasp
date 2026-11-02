//! Named playlists stored in the library database.

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::library::Track;

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: u32,
    /// Owning folder, or `None` when the playlist sits at the root.
    pub folder_id: Option<i64>,
}

/// A user-created folder for grouping playlists. Folders may be nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistFolder {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
}

pub fn list_folders(conn: &Connection) -> Result<Vec<PlaylistFolder>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, parent_id
         FROM playlist_folders
         ORDER BY name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PlaylistFolder {
            id: r.get(0)?,
            name: r.get(1)?,
            parent_id: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn create_folder(conn: &Connection, name: &str, parent_id: Option<i64>) -> Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "folder name cannot be empty");
    if let Some(parent) = parent_id {
        anyhow::ensure!(folder_exists(conn, parent)?, "parent folder does not exist");
    }
    conn.execute(
        "INSERT INTO playlist_folders (name, parent_id) VALUES (?1, ?2)",
        params![name, parent_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn rename_folder(conn: &Connection, folder_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "folder name cannot be empty");
    let changed = conn.execute(
        "UPDATE playlist_folders SET name = ?1 WHERE id = ?2",
        params![name, folder_id],
    )?;
    anyhow::ensure!(changed > 0, "folder does not exist");
    Ok(())
}

/// Delete a folder without losing what it contains: child playlists and child
/// folders move up to the deleted folder's parent (the root for top folders).
pub fn delete_folder(conn: &Connection, folder_id: i64) -> Result<()> {
    let parent_id: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM playlist_folders WHERE id = ?1",
            params![folder_id],
            |r| r.get(0),
        )
        .map_err(|_| anyhow::anyhow!("folder does not exist"))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE playlists SET folder_id = ?1 WHERE folder_id = ?2",
        params![parent_id, folder_id],
    )?;
    tx.execute(
        "UPDATE smart_playlists SET folder_id = ?1 WHERE folder_id = ?2",
        params![parent_id, folder_id],
    )?;
    tx.execute(
        "UPDATE playlist_folders SET parent_id = ?1 WHERE parent_id = ?2",
        params![parent_id, folder_id],
    )?;
    tx.execute(
        "DELETE FROM playlist_folders WHERE id = ?1",
        params![folder_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Put a playlist in `folder_id`, or back at the root when it is `None`.
pub fn set_folder(conn: &Connection, playlist_id: i64, folder_id: Option<i64>) -> Result<()> {
    if let Some(folder) = folder_id {
        anyhow::ensure!(folder_exists(conn, folder)?, "folder does not exist");
    }
    let changed = conn.execute(
        "UPDATE playlists SET folder_id = ?1 WHERE id = ?2",
        params![folder_id, playlist_id],
    )?;
    anyhow::ensure!(changed > 0, "playlist does not exist");
    Ok(())
}

pub fn move_playlist_to_folder(
    conn: &Connection,
    playlist_id: i64,
    folder_id: Option<i64>,
) -> Result<()> {
    set_folder(conn, playlist_id, folder_id)
}

pub(crate) fn folder_exists(conn: &Connection, folder_id: i64) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM playlist_folders WHERE id = ?1",
        params![folder_id],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

pub fn create(conn: &Connection, name: &str, track_ids: &[i64]) -> Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "playlist name cannot be empty");
    conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name])?;
    let id = conn.last_insert_rowid();
    for (pos, track_id) in track_ids.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3)",
            params![id, track_id, pos as i64],
        )?;
    }
    Ok(id)
}

pub fn list(conn: &Connection) -> Result<Vec<Playlist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(pt.track_id), p.folder_id
         FROM playlists p
         LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
         GROUP BY p.id
         ORDER BY p.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Playlist {
            id: r.get(0)?,
            name: r.get(1)?,
            count: r.get(2)?,
            folder_id: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn tracks(conn: &Connection, playlist_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {}
         FROM playlist_tracks pt
         JOIN tracks t ON t.id = pt.track_id
         WHERE pt.playlist_id = ?1
         ORDER BY pt.position",
        crate::library::TRACK_COLUMNS
    ))?;
    let rows = stmt.query_map(params![playlist_id], crate::library::row_to_track)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn add_track(conn: &Connection, playlist_id: i64, track_id: i64) -> Result<()> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
         VALUES (?1, ?2, ?3)",
        params![playlist_id, track_id, next],
    )?;
    Ok(())
}

/// Append several tracks in one transaction, ignoring duplicates.
pub fn add_tracks(conn: &Connection, playlist_id: i64, track_ids: &[i64]) -> Result<usize> {
    let mut next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |r| r.get(0),
    )?;
    let mut added = 0;
    for track_id in track_ids {
        let changed = conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3)",
            params![playlist_id, track_id, next],
        )?;
        if changed > 0 {
            added += 1;
            next += 1;
        }
    }
    Ok(added)
}

/// Remove tracks from a playlist, keeping the remaining order.
pub fn remove_tracks(conn: &Connection, playlist_id: i64, track_ids: &[i64]) -> Result<usize> {
    if track_ids.is_empty() {
        return Ok(0);
    }
    let mut removed = 0;
    for id in track_ids {
        removed += conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, id],
        )?;
    }
    compact(conn, playlist_id)?;
    Ok(removed)
}

/// Move `moving` so it sits directly before `before`, or at the end when
/// `before` is `None`. `moving` keeps its relative order.
pub fn move_tracks(
    conn: &Connection,
    playlist_id: i64,
    moving: &[i64],
    before: Option<i64>,
) -> Result<()> {
    if moving.is_empty() {
        return Ok(());
    }

    let mut order = track_order(conn, playlist_id)?;
    let moving_set: std::collections::HashSet<i64> = moving.iter().copied().collect();

    // Drop the moved items, then insert them at the requested spot.
    order.retain(|id| !moving_set.contains(id));
    let insert_at = before
        .and_then(|b| order.iter().position(|&id| id == b))
        .unwrap_or(order.len());
    let mut block: Vec<i64> = moving
        .iter()
        .copied()
        .filter(|id| moving_set.contains(id))
        .collect();
    block.dedup();
    order.splice(insert_at..insert_at, block);

    write_order(conn, playlist_id, &order)?;
    Ok(())
}

/// Rewrite the whole order (used by tests and by callers that computed it).
pub fn set_order(conn: &Connection, playlist_id: i64, order: &[i64]) -> Result<()> {
    write_order(conn, playlist_id, order)
}

pub fn track_order(conn: &Connection, playlist_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map(params![playlist_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn write_order(conn: &Connection, playlist_id: i64, order: &[i64]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (position, track_id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_tracks SET position = ?1 WHERE playlist_id = ?2 AND track_id = ?3",
            params![position as i64, playlist_id, track_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Close gaps left by removals so positions stay 0..n.
fn compact(conn: &Connection, playlist_id: i64) -> Result<()> {
    let order = track_order(conn, playlist_id)?;
    write_order(conn, playlist_id, &order)?;
    Ok(())
}

pub fn delete(conn: &Connection, playlist_id: i64) -> Result<()> {
    conn.execute("DELETE FROM playlists WHERE id = ?1", params![playlist_id])?;
    Ok(())
}

pub fn rename(conn: &Connection, playlist_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "playlist name cannot be empty");
    conn.execute(
        "UPDATE playlists SET name = ?1 WHERE id = ?2",
        params![name, playlist_id],
    )?;
    Ok(())
}
