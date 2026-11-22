//! Internet radio stations.

use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct RadioStation {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub genre: String,
    pub image_url: String,
}

pub fn stations(conn: &Connection) -> Result<Vec<RadioStation>> {
    let mut statement = conn.prepare(
        "SELECT id, name, url, genre, image_url FROM radio_stations ORDER BY LOWER(name), id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RadioStation {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            genre: row.get(3)?,
            image_url: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Add a station, or update its name/genre when the URL already exists.
pub fn add(conn: &Connection, name: &str, url: &str, genre: &str, image_url: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO radio_stations (name, url, genre, image_url) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(url) DO UPDATE SET
             name = excluded.name,
             genre = excluded.genre,
             image_url = CASE WHEN excluded.image_url <> '' THEN excluded.image_url
                              ELSE radio_stations.image_url END",
        params![name.trim(), url.trim(), genre.trim(), image_url.trim()],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM radio_stations WHERE url = ?1",
        [url.trim()],
        |row| row.get(0),
    )?)
}

pub fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    url: &str,
    genre: &str,
    image_url: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE radio_stations SET name = ?1, url = ?2, genre = ?3, image_url = ?4 WHERE id = ?5",
        params![name.trim(), url.trim(), genre.trim(), image_url.trim(), id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM radio_stations WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_artwork_round_trips_and_is_not_cleared_by_re_add() {
        let conn = crate::db::open_in_memory().unwrap();
        let id = add(
            &conn,
            "FIP",
            "https://example.com/stream.mp3",
            "Eclectic",
            "https://example.com/fip.png",
        )
        .unwrap();
        assert_eq!(
            stations(&conn).unwrap()[0].image_url,
            "https://example.com/fip.png"
        );

        // Re-adding the same URL without a cover keeps the existing one.
        add(
            &conn,
            "FIP",
            "https://example.com/stream.mp3",
            "Eclectic",
            "",
        )
        .unwrap();
        assert_eq!(
            stations(&conn).unwrap()[0].image_url,
            "https://example.com/fip.png"
        );

        update(
            &conn,
            id,
            "FIP",
            "https://example.com/stream.mp3",
            "Eclectic",
            "https://example.com/new.png",
        )
        .unwrap();
        assert_eq!(
            stations(&conn).unwrap()[0].image_url,
            "https://example.com/new.png"
        );
    }
}
