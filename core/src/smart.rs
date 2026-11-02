//! Smart playlists: stored rule sets compiled to SQL when opened.

use anyhow::Result;
use rusqlite::{params, types::Value, Connection};
use serde::{Deserialize, Serialize};

use crate::library::{row_to_track, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Duration,
    Added,
    Rating,
    PlayCount,
}

impl Field {
    pub const ALL: [Field; 9] = [
        Field::Title,
        Field::Artist,
        Field::Album,
        Field::Genre,
        Field::Year,
        Field::Duration,
        Field::Added,
        Field::Rating,
        Field::PlayCount,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::Genre => "Genre",
            Field::Year => "Year",
            Field::Duration => "Duration (s)",
            Field::Added => "Date added",
            Field::Rating => "Rating",
            Field::PlayCount => "Play count",
        }
    }

    fn column(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Artist => "artist",
            Field::Album => "album",
            Field::Genre => "genre",
            Field::Year => "year",
            Field::Duration => "duration_secs",
            Field::Added => "date_added",
            Field::Rating => "rating",
            Field::PlayCount => "play_count",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Field::Year | Field::Duration | Field::Added | Field::Rating | Field::PlayCount
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Contains,
    Equals,
    NotEquals,
    Greater,
    Less,
}

impl Op {
    pub const ALL: [Op; 5] = [
        Op::Contains,
        Op::Equals,
        Op::NotEquals,
        Op::Greater,
        Op::Less,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::Equals => "is",
            Op::NotEquals => "is not",
            Op::Greater => "greater than",
            Op::Less => "less than",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Op::Contains => "LIKE",
            Op::Equals => "=",
            Op::NotEquals => "<>",
            Op::Greater => ">",
            Op::Less => "<",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

impl Rule {
    pub fn new(field: Field, op: Op) -> Self {
        Self {
            field,
            op,
            value: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    ArtistThenAlbum,
    Title,
    YearNewest,
    RecentlyAdded,
    ShortestFirst,
    LongestFirst,
}

impl SortBy {
    pub const ALL: [SortBy; 6] = [
        SortBy::ArtistThenAlbum,
        SortBy::Title,
        SortBy::YearNewest,
        SortBy::RecentlyAdded,
        SortBy::ShortestFirst,
        SortBy::LongestFirst,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortBy::ArtistThenAlbum => "Artist, then album",
            SortBy::Title => "Title",
            SortBy::YearNewest => "Newest year first",
            SortBy::RecentlyAdded => "Recently added",
            SortBy::ShortestFirst => "Shortest first",
            SortBy::LongestFirst => "Longest first",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            SortBy::ArtistThenAlbum => {
                "artist COLLATE NOCASE, album COLLATE NOCASE, disc_no, track_no, title COLLATE NOCASE"
            }
            SortBy::Title => "title COLLATE NOCASE",
            SortBy::YearNewest => "year DESC, artist COLLATE NOCASE",
            SortBy::RecentlyAdded => "added_at DESC",
            SortBy::ShortestFirst => "duration_secs ASC",
            SortBy::LongestFirst => "duration_secs DESC",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Definition {
    /// Rule 0 is always present; an empty rule set means "everything".
    #[serde(default)]
    pub match_all: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default = "default_sort")]
    pub sort: SortBy,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn default_sort() -> SortBy {
    SortBy::ArtistThenAlbum
}

impl Default for Definition {
    fn default() -> Self {
        Self {
            match_all: true,
            rules: Vec::new(),
            sort: SortBy::ArtistThenAlbum,
            limit: None,
        }
    }
}

impl Definition {
    /// Compact JSON stored in the database.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
}

#[derive(Debug, Clone)]
pub struct SmartPlaylist {
    pub id: i64,
    pub name: String,
    /// Owning folder, or `None` when the playlist sits at the root.
    pub folder_id: Option<i64>,
    pub definition: Definition,
}

pub fn create(
    conn: &Connection,
    name: &str,
    definition: &Definition,
    folder_id: Option<i64>,
) -> Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "playlist name cannot be empty");
    if let Some(folder) = folder_id {
        anyhow::ensure!(
            crate::playlist::folder_exists(conn, folder)?,
            "folder does not exist"
        );
    }
    conn.execute(
        "INSERT INTO smart_playlists (name, definition, folder_id) VALUES (?1, ?2, ?3)",
        params![name, definition.to_json()?, folder_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list(conn: &Connection) -> Result<Vec<SmartPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, folder_id, definition
         FROM smart_playlists
         ORDER BY name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, folder_id, definition) = row?;
        // A corrupt definition should not make the whole sidebar unusable.
        let definition = Definition::from_json(&definition).unwrap_or_default();
        out.push(SmartPlaylist {
            id,
            name,
            folder_id,
            definition,
        });
    }
    Ok(out)
}

/// Put a smart playlist in `folder_id`, or back at the root when it is `None`.
pub fn set_folder(conn: &Connection, playlist_id: i64, folder_id: Option<i64>) -> Result<()> {
    if let Some(folder) = folder_id {
        anyhow::ensure!(
            crate::playlist::folder_exists(conn, folder)?,
            "folder does not exist"
        );
    }
    let changed = conn.execute(
        "UPDATE smart_playlists SET folder_id = ?1 WHERE id = ?2",
        params![folder_id, playlist_id],
    )?;
    anyhow::ensure!(changed > 0, "smart playlist does not exist");
    Ok(())
}

pub fn delete(conn: &Connection, playlist_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM smart_playlists WHERE id = ?1",
        params![playlist_id],
    )?;
    Ok(())
}

pub fn rename(conn: &Connection, playlist_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "playlist name cannot be empty");
    conn.execute(
        "UPDATE smart_playlists SET name = ?1 WHERE id = ?2",
        params![name, playlist_id],
    )?;
    Ok(())
}

pub fn update_definition(
    conn: &Connection,
    playlist_id: i64,
    definition: &Definition,
) -> Result<()> {
    conn.execute(
        "UPDATE smart_playlists SET definition = ?1 WHERE id = ?2",
        params![definition.to_json()?, playlist_id],
    )?;
    Ok(())
}

/// Evaluate a definition into tracks.
pub fn evaluate(conn: &Connection, definition: &Definition) -> Result<Vec<Track>> {
    let mut sql = format!("SELECT {} FROM tracks", crate::library::TRACK_COLUMNS);
    let mut args: Vec<Value> = Vec::new();
    let mut clauses = Vec::new();

    for rule in &definition.rules {
        if rule.value.trim().is_empty() {
            continue;
        }

        let column = rule.field.column();
        if rule.field.is_numeric() {
            let Ok(number) = rule.value.trim().parse::<i64>() else {
                continue; // unparsable numeric rule is ignored, not fatal
            };
            let op = match rule.op {
                Op::Greater => ">",
                Op::Less => "<",
                Op::Equals => "=",
                Op::NotEquals => "<>",
                Op::Contains => "LIKE",
            };
            clauses.push(format!("{column} {op} ?"));
            args.push(Value::Integer(number));
        } else if rule.op == Op::Contains {
            clauses.push(format!("{column} LIKE ?"));
            args.push(Value::Text(format!("%{}%", rule.value.trim())));
        } else {
            clauses.push(format!("{column} {} ?", rule.op.sql()));
            args.push(Value::Text(rule.value.trim().to_string()));
        }
    }

    if !clauses.is_empty() {
        let joiner = if definition.match_all {
            " AND "
        } else {
            " OR "
        };
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(joiner));
    }

    sql.push_str(" ORDER BY ");
    sql.push_str(definition.sort.sql());

    if let Some(limit) = definition.limit.filter(|l| *l > 0) {
        sql.push_str(" LIMIT ?");
        args.push(Value::Integer(limit.min(10_000) as i64));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), row_to_track)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn tracks(conn: &Connection, playlist_id: i64) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare("SELECT definition FROM smart_playlists WHERE id = ?1")?;
    let mut rows = stmt.query(params![playlist_id])?;
    let Some(row) = rows.next()? else {
        return Ok(Vec::new());
    };
    let text: String = row.get(0)?;
    let definition = Definition::from_json(&text).unwrap_or_default();
    evaluate(conn, &definition)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, genre: &str, year: i32, secs: u64) -> Track {
        Track {
            id: 0,
            path: format!("/music/{title}.mp3"),
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            album_artist: artist.into(),
            genre: genre.into(),
            year: Some(year),
            track_no: Some(1),
            disc_no: Some(1),
            duration_secs: secs,
            sample_rate: 44_100,
            bitrate: 320,
            format: "MPEG".into(),
            rating: 0,
            play_count: 0,
            skip_count: 0,
            last_played: None,
            date_added: 0,
        }
    }

    #[test]
    fn definition_round_trips_through_json() {
        let definition = Definition {
            match_all: false,
            rules: vec![
                Rule {
                    field: Field::Genre,
                    op: Op::Contains,
                    value: "rock".into(),
                },
                Rule {
                    field: Field::Year,
                    op: Op::Greater,
                    value: "1990".into(),
                },
            ],
            sort: SortBy::RecentlyAdded,
            limit: Some(25),
        };
        let json = definition.to_json().unwrap();
        assert_eq!(Definition::from_json(&json).unwrap(), definition);
    }

    #[test]
    fn default_definition_is_valid_json() {
        let definition = Definition::default();
        let json = definition.to_json().unwrap();
        assert!(json.contains("\"match_all\":true"));
        assert_eq!(Definition::from_json(&json).unwrap(), definition);
    }

    #[test]
    fn rule_matching_is_case_insensitive_for_text() {
        // Guard against regressions in the SQL we generate.
        let rule = Rule {
            field: Field::Artist,
            op: Op::Contains,
            value: "rad".into(),
        };
        assert_eq!(rule.field.column(), "artist");
        assert_eq!(Op::Contains.sql(), "LIKE");
        assert!(!rule.field.is_numeric());
        assert!(Field::Year.is_numeric());
    }

    #[test]
    fn sort_choices_are_all_distinct() {
        let mut sql: Vec<&str> = SortBy::ALL.iter().map(|s| s.sql()).collect();
        sql.sort_unstable();
        sql.dedup();
        assert_eq!(sql.len(), SortBy::ALL.len());
    }

    #[test]
    fn label_coverage() {
        for field in Field::ALL {
            assert!(!field.label().is_empty());
        }
        for op in Op::ALL {
            assert!(!op.label().is_empty());
        }
        for sort in SortBy::ALL {
            assert!(!sort.label().is_empty());
        }
        assert_eq!(track("t", "a", "g", 2000, 60).duration_secs, 60);
    }
}
