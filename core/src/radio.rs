//! Internet radio stations.

use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct RadioStation {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub genre: String,
}

pub fn stations(conn: &Connection) -> Result<Vec<RadioStation>> {
    let mut statement =
        conn.prepare("SELECT id, name, url, genre FROM radio_stations ORDER BY LOWER(name), id")?;
    let rows = statement.query_map([], |row| {
        Ok(RadioStation {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            genre: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Add a station, or update its name/genre when the URL already exists.
pub fn add(conn: &Connection, name: &str, url: &str, genre: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO radio_stations (name, url, genre) VALUES (?1, ?2, ?3)
         ON CONFLICT(url) DO UPDATE SET name = excluded.name, genre = excluded.genre",
        params![name.trim(), url.trim(), genre.trim()],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM radio_stations WHERE url = ?1",
        [url.trim()],
        |row| row.get(0),
    )?)
}

pub fn update(conn: &Connection, id: i64, name: &str, url: &str, genre: &str) -> Result<()> {
    conn.execute(
        "UPDATE radio_stations SET name = ?1, url = ?2, genre = ?3 WHERE id = ?4",
        params![name.trim(), url.trim(), genre.trim(), id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM radio_stations WHERE id = ?1", [id])?;
    Ok(())
}
