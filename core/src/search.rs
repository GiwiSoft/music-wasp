//! Advanced search syntax and saved searches.
//!
//! The search box accepts plain text plus field filters:
//!
//! ```text
//! artist:"radiohead" year:1997
//! genre:rock year:>=1990
//! album:"OK Computer" title:"airbag"
//! ```
//!
//! Filters are ANDed. Text fields match substrings unless the value starts
//! with `=`, which makes the match exact.

use anyhow::Result;
use rusqlite::{params, Connection};

/// Text columns that can be searched with `field:value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Format,
}

impl TextField {
    pub fn column(self) -> &'static str {
        match self {
            TextField::Title => "title",
            TextField::Artist => "artist",
            TextField::Album => "album",
            TextField::AlbumArtist => "album_artist",
            TextField::Genre => "genre",
            TextField::Format => "format",
        }
    }
}

/// Numeric columns that can be searched with `field:number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberField {
    Year,
    Track,
    Disc,
    Duration,
}

impl NumberField {
    pub fn column(self) -> &'static str {
        match self {
            NumberField::Year => "year",
            NumberField::Track => "track_no",
            NumberField::Disc => "disc_no",
            NumberField::Duration => "duration_secs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberOp {
    Equal,
    Greater,
    GreaterEq,
    Less,
    LessEq,
}

impl NumberOp {
    pub fn sql(self) -> &'static str {
        match self {
            NumberOp::Equal => "=",
            NumberOp::Greater => ">",
            NumberOp::GreaterEq => ">=",
            NumberOp::Less => "<",
            NumberOp::LessEq => "<=",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// `artist:radiohead`, or exact when `exact` is true (`artist:=radiohead`).
    Text {
        field: TextField,
        value: String,
        exact: bool,
    },
    /// `year:1997`, `year:>=1990`, `track:<10`.
    Number {
        field: NumberField,
        op: NumberOp,
        value: i64,
    },
    /// `year:1990..1999`.
    Range {
        field: NumberField,
        from: i64,
        to: i64,
    },
}

/// Parsed search: free text plus field filters, all ANDed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub conditions: Vec<Condition>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.conditions.is_empty()
    }
}

/// Parse an advanced search query. Unknown fields remain plain text.
pub fn parse(input: &str) -> Query {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut free = Vec::new();
    let mut conditions = Vec::new();

    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        if chars[i] == '"' {
            let (value, next) = read_quoted(&chars, i);
            free.push(value);
            i = next;
            continue;
        }

        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != ':' {
            i += 1;
        }
        if i < chars.len() && chars[i] == ':' && i > start {
            let key: String = chars[start..i].iter().collect();
            let field = if let Some(field) = text_field(&key) {
                Some(FieldFilter::Text(field))
            } else {
                number_field(&key).map(FieldFilter::Number)
            };
            if let Some(field) = field {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let (raw, next) = if i < chars.len() && chars[i] == '"' {
                    read_quoted(&chars, i)
                } else {
                    read_bare(&chars, i)
                };
                i = next;
                if raw.is_empty() {
                    free.push(format!("{key}:"));
                    continue;
                }
                match field {
                    FieldFilter::Text(field) => {
                        let (value, exact) = match raw.strip_prefix('=') {
                            Some(value) if !value.is_empty() => (value.to_string(), true),
                            _ => (raw, false),
                        };
                        conditions.push(Condition::Text {
                            field,
                            value,
                            exact,
                        });
                    }
                    FieldFilter::Number(field) => match parse_number_condition(field, &raw) {
                        Some(condition) => conditions.push(condition),
                        None => free.push(format!("{key}:{raw}")),
                    },
                }
                continue;
            }
        }

        // Not a known field: read the token as ordinary free text.
        i = start;
        if chars[i] == '"' {
            let (value, next) = read_quoted(&chars, i);
            free.push(value);
            i = next;
        } else {
            let (token, next) = read_bare(&chars, i);
            free.push(token);
            i = next;
        }
    }

    Query {
        text: free.join(" "),
        conditions,
    }
}

enum FieldFilter {
    Text(TextField),
    Number(NumberField),
}

fn text_field(key: &str) -> Option<TextField> {
    match key.to_ascii_lowercase().as_str() {
        "title" => Some(TextField::Title),
        "artist" => Some(TextField::Artist),
        "album" => Some(TextField::Album),
        "albumartist" | "album_artist" | "album-artist" => Some(TextField::AlbumArtist),
        "genre" => Some(TextField::Genre),
        "format" | "ext" => Some(TextField::Format),
        _ => None,
    }
}

fn number_field(key: &str) -> Option<NumberField> {
    match key.to_ascii_lowercase().as_str() {
        "year" => Some(NumberField::Year),
        "track" | "trackno" | "track_no" | "track-number" => Some(NumberField::Track),
        "disc" | "discno" | "disc_no" | "disc-number" => Some(NumberField::Disc),
        "duration" | "length" => Some(NumberField::Duration),
        _ => None,
    }
}

fn parse_number_condition(field: NumberField, raw: &str) -> Option<Condition> {
    let raw = raw.trim();
    if let Some((from, to)) = raw.split_once("..") {
        return Some(Condition::Range {
            field,
            from: from.parse().ok()?,
            to: to.parse().ok()?,
        });
    }
    let (op, value) = if let Some(value) = raw.strip_prefix(">=") {
        (NumberOp::GreaterEq, value)
    } else if let Some(value) = raw.strip_prefix("<=") {
        (NumberOp::LessEq, value)
    } else if let Some(value) = raw.strip_prefix('>') {
        (NumberOp::Greater, value)
    } else if let Some(value) = raw.strip_prefix('<') {
        (NumberOp::Less, value)
    } else if let Some(value) = raw.strip_prefix('=') {
        (NumberOp::Equal, value)
    } else {
        (NumberOp::Equal, raw)
    };
    Some(Condition::Number {
        field,
        op,
        value: value.parse().ok()?,
    })
}

fn read_quoted(chars: &[char], start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '"' => {
                return (out, i + 1);
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    (out, i)
}

fn read_bare(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    (chars[start..i].iter().collect(), i)
}

// ---- saved searches ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub query: String,
}

pub fn list_saved(conn: &Connection) -> Result<Vec<SavedSearch>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, query
         FROM saved_searches
         ORDER BY name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SavedSearch {
            id: r.get(0)?,
            name: r.get(1)?,
            query: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn create_saved(conn: &Connection, name: &str, query: &str) -> Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "saved search name cannot be empty");
    let query = query.trim();
    anyhow::ensure!(!query.is_empty(), "saved search query cannot be empty");
    conn.execute(
        "INSERT INTO saved_searches (name, query) VALUES (?1, ?2)",
        params![name, query],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_saved(conn: &Connection, id: i64, name: &str, query: &str) -> Result<()> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "saved search name cannot be empty");
    let query = query.trim();
    anyhow::ensure!(!query.is_empty(), "saved search query cannot be empty");
    let changed = conn.execute(
        "UPDATE saved_searches SET name = ?1, query = ?2 WHERE id = ?3",
        params![name, query, id],
    )?;
    anyhow::ensure!(changed > 0, "saved search does not exist");
    Ok(())
}

pub fn delete_saved(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute("DELETE FROM saved_searches WHERE id = ?1", params![id])?;
    anyhow::ensure!(changed > 0, "saved search does not exist");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_and_quoted_values() {
        let query = parse("artist:\"radiohead\" year:1997");
        assert_eq!(query.text, "");
        assert_eq!(
            query.conditions,
            vec![
                Condition::Text {
                    field: TextField::Artist,
                    value: "radiohead".to_string(),
                    exact: false,
                },
                Condition::Number {
                    field: NumberField::Year,
                    op: NumberOp::Equal,
                    value: 1997,
                },
            ]
        );
    }

    #[test]
    fn keeps_unknown_fields_as_text_and_understands_escapes() {
        let query = parse("foo:bar \"dark \\\"side\\\"\" title:=airbag");
        assert_eq!(query.text, "foo:bar dark \"side\"");
        assert_eq!(
            query.conditions,
            vec![Condition::Text {
                field: TextField::Title,
                value: "airbag".to_string(),
                exact: true,
            }]
        );
    }

    #[test]
    fn parses_numeric_operators_and_ranges() {
        assert_eq!(
            parse("year:>=1990").conditions,
            vec![Condition::Number {
                field: NumberField::Year,
                op: NumberOp::GreaterEq,
                value: 1990,
            }]
        );
        assert_eq!(
            parse("year:1990..1999").conditions,
            vec![Condition::Range {
                field: NumberField::Year,
                from: 1990,
                to: 1999,
            }]
        );
        assert_eq!(
            parse("track:<10 and disc:1").conditions,
            vec![
                Condition::Number {
                    field: NumberField::Track,
                    op: NumberOp::Less,
                    value: 10,
                },
                Condition::Number {
                    field: NumberField::Disc,
                    op: NumberOp::Equal,
                    value: 1,
                },
            ]
        );
    }
}
