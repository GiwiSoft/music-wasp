//! M3U / M3U8 playlist import and export.
//!
//! Import resolves paths relative to the playlist file, indexes audio files
//! that are not in the library yet, and creates a normal editable playlist.
//! Export writes an `#EXTM3U` playlist with one absolute path per track.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::library::{is_audio_path, refresh_file, Track};
use crate::{playlist, smart};

/// One playable line in an M3U file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// Title from `#EXTINF`, when present.
    pub title: Option<String>,
    pub duration_secs: Option<u64>,
}

/// Result of importing a playlist file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub playlist_id: i64,
    pub name: String,
    /// Path lines found in the file, including duplicates and missing files.
    pub entries: usize,
    /// Unique tracks added to the new playlist.
    pub added: usize,
    /// Paths that do not exist on disk.
    pub missing: usize,
    /// URLs, non-audio files and other lines M3U can hold but the library cannot.
    pub unsupported: usize,
}

/// Does this file name look like an M3U playlist?
pub fn is_m3u_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("m3u") || ext.eq_ignore_ascii_case("m3u8"))
}

/// Parse M3U text. Relative paths are resolved against `base`; comments,
/// `#EXTINF` metadata and blank lines are understood. URLs are skipped.
pub fn parse_m3u(text: &str, base: &Path) -> Vec<Entry> {
    parse_m3u_report(text, base).0
}

fn parse_m3u_report(text: &str, base: &Path) -> (Vec<Entry>, usize) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut unsupported = 0usize;
    let mut pending_title: Option<String> = None;
    let mut pending_duration: Option<u64> = None;

    for raw in text.lines() {
        let line = raw.trim().trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(meta) = line.strip_prefix("#EXTINF:") {
            let (duration, title) = meta.split_once(',').unwrap_or((meta, ""));
            pending_duration = duration.trim().parse::<u64>().ok();
            let title = title.trim();
            pending_title = if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            };
            continue;
        }
        if line.starts_with('#') {
            continue;
        }

        match resolve_line(line, base) {
            Some(path) => entries.push(Entry {
                path,
                title: pending_title.take(),
                duration_secs: pending_duration.take(),
            }),
            None => {
                unsupported += 1;
                pending_title = None;
                pending_duration = None;
            }
        }
    }

    (entries, unsupported)
}

fn resolve_line(line: &str, base: &Path) -> Option<PathBuf> {
    let raw = line.trim().trim_matches('"');
    if raw.is_empty() || is_url(raw) {
        return None;
    }

    if let Some(uri) = raw.strip_prefix("file://") {
        let decoded = percent_decode(uri);
        let path = PathBuf::from(decoded);
        return Some(path);
    }

    let direct = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        base.join(raw)
    };

    // Playlists exported on Windows often use backslashes. Treat them as
    // separators when the literal path does not exist on this platform.
    if !direct.exists() && raw.contains('\\') && std::path::MAIN_SEPARATOR != '\\' {
        let alt = raw.replace('\\', "/");
        return Some(if Path::new(&alt).is_absolute() {
            PathBuf::from(alt)
        } else {
            base.join(alt)
        });
    }

    Some(direct)
}

fn is_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["http://", "https://", "ftp://", "rtsp://", "mms://"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Read and parse a playlist file. Invalid UTF-8 is read lossily so old
/// Windows-encoded `.m3u` files still import their paths.
pub fn read_m3u(path: &Path) -> Result<Vec<Entry>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read playlist {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parse_m3u(&text, base))
}

/// Import an M3U file as a new playlist, indexing audio files on the way.
/// `name` overrides the playlist name; otherwise the file stem is used.
/// `folder_id` places the playlist inside a playlist folder.
pub fn import(
    conn: &Connection,
    path: &Path,
    name: Option<&str>,
    folder_id: Option<i64>,
) -> Result<ImportReport> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read playlist {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let (entries, mut unsupported) = parse_m3u_report(&text, base);
    let entry_count = entries.len() + unsupported;

    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    let mut missing = 0usize;
    for entry in entries {
        if !entry.path.is_file() {
            missing += 1;
            continue;
        }
        if !is_audio_path(&entry.path) {
            unsupported += 1;
            continue;
        }
        match track_id_for_path(conn, &entry.path)? {
            Some(id) => {
                if seen.insert(id) {
                    ids.push(id);
                }
            }
            None => {
                if let Err(error) = refresh_file(conn, &entry.path) {
                    tracing::warn!("skip {}: {error}", entry.path.display());
                    missing += 1;
                    continue;
                }
                if let Some(id) = track_id_for_path(conn, &entry.path)? {
                    if seen.insert(id) {
                        ids.push(id);
                    }
                } else {
                    missing += 1;
                }
            }
        }
    }

    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Imported playlist".to_string());

    let playlist_id = playlist::create(conn, &name, &ids)?;
    if folder_id.is_some() {
        playlist::set_folder(conn, playlist_id, folder_id)?;
    }
    let added = playlist::track_order(conn, playlist_id)?.len();

    Ok(ImportReport {
        playlist_id,
        name,
        entries: entry_count,
        added,
        missing,
        unsupported,
    })
}

fn track_id_for_path(conn: &Connection, path: &Path) -> Result<Option<i64>> {
    let text = path.to_string_lossy();
    let exact: Option<i64> = conn
        .query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            params![text],
            |r| r.get(0),
        )
        .ok();
    if exact.is_some() {
        return Ok(exact);
    }

    // An existing row may use a non-canonical spelling (relative path, `..`,
    // symlink). Match it by canonical path before inserting a duplicate.
    let wanted = std::fs::canonicalize(path).ok();
    let Some(wanted) = wanted else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT id, path FROM tracks")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, stored) = row?;
        if std::fs::canonicalize(stored).ok().as_deref() == Some(wanted.as_path()) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Render a manual playlist as M3U text.
pub fn export_text(conn: &Connection, playlist_id: i64) -> Result<String> {
    let tracks = playlist::tracks(conn, playlist_id)?;
    let name = playlist::list(conn)?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .map(|playlist| playlist.name)
        .ok_or_else(|| anyhow::anyhow!("playlist does not exist"))?;
    Ok(render(&name, &tracks))
}

/// Render a smart playlist as M3U text, evaluating its current rules.
pub fn export_smart_text(conn: &Connection, playlist_id: i64) -> Result<String> {
    let name = smart::list(conn)?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .map(|playlist| playlist.name)
        .ok_or_else(|| anyhow::anyhow!("smart playlist does not exist"))?;
    let tracks = smart::tracks(conn, playlist_id)?;
    Ok(render(&name, &tracks))
}

/// Shared `#EXTM3U` rendering with absolute paths and `#EXTINF` metadata.
fn render(name: &str, tracks: &[Track]) -> String {
    let mut out = String::from("#EXTM3U\n");
    out.push_str(&format!("#PLAYLIST:{name}\n"));
    for track in tracks {
        let title = match (track.artist.trim(), track.title.trim()) {
            ("", "") => track
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&track.path)
                .to_string(),
            ("", title) => title.to_string(),
            (artist, "") => artist.to_string(),
            (artist, title) => format!("{artist} - {title}"),
        };
        out.push_str(&format!(
            "#EXTINF:{},{}\n{}\n",
            track.duration_secs, title, track.path
        ));
    }
    out
}

/// Write text to an M3U/M3U8 file. Returns the number of tracks written.
fn write(text: &str, path: &Path) -> Result<usize> {
    let count = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .count();
    std::fs::write(path, text)
        .with_context(|| format!("cannot write playlist {}", path.display()))?;
    Ok(count)
}

/// Write a manual playlist to an M3U/M3U8 file.
pub fn export(conn: &Connection, playlist_id: i64, path: &Path) -> Result<usize> {
    write(&export_text(conn, playlist_id)?, path)
}

/// Write a smart playlist to an M3U/M3U8 file.
pub fn export_smart(conn: &Connection, playlist_id: i64, path: &Path) -> Result<usize> {
    write(&export_smart_text(conn, playlist_id)?, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_extinf_and_relative_paths() {
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:123,Artist - Song\r\nmusic/song.mp3\r\n\
                    missing.flac\n# comment\nhttps://radio.example/stream\n";
        let parsed = parse_m3u(text, Path::new("/library"));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].path, PathBuf::from("/library/music/song.mp3"));
        assert_eq!(parsed[0].duration_secs, Some(123));
        assert_eq!(parsed[0].title.as_deref(), Some("Artist - Song"));
        assert_eq!(parsed[1].path, PathBuf::from("/library/missing.flac"));
    }

    #[test]
    fn resolves_file_urls_and_windows_separators() {
        let parsed = parse_m3u(
            "file:///music/My%20Song.mp3\nmusic\\song.flac\n",
            Path::new("/library"),
        );
        assert_eq!(parsed[0].path, PathBuf::from("/music/My Song.mp3"));
        assert_eq!(parsed[1].path, PathBuf::from("/library/music/song.flac"));
    }
}
