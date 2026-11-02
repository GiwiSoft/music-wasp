//! Move or rename audio files from their tags.
//!
//! [`plan`] renders a configurable pattern into target paths and reports
//! collisions without touching the disk. [`apply`] performs the safe rows and
//! returns a journal that [`undo`] can play back, restoring both the files and
//! their library rows.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::library::Track;

/// Default folder/filename template.
pub const DEFAULT_PATTERN: &str = "{artist}/{album}/{track:02} - {title}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveStatus {
    Ready,
    Unchanged,
    Collision,
    Missing,
    Invalid,
}

impl MoveStatus {
    pub fn label(self) -> &'static str {
        match self {
            MoveStatus::Ready => "move",
            MoveStatus::Unchanged => "unchanged",
            MoveStatus::Collision => "collision",
            MoveStatus::Missing => "missing",
            MoveStatus::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizeRow {
    pub track_id: i64,
    pub title: String,
    pub current_path: PathBuf,
    pub target_path: PathBuf,
    pub status: MoveStatus,
    /// Why the row was skipped, or an empty string when it is ready.
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OrganizePlan {
    pub rows: Vec<OrganizeRow>,
    pub ready: usize,
    pub skipped: usize,
}

impl OrganizePlan {
    pub fn has_moves(&self) -> bool {
        self.ready > 0
    }
}

/// One completed move, kept so the batch can be undone later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRecord {
    pub track_id: i64,
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ApplyReport {
    pub moved: usize,
    pub failed: Vec<String>,
    pub records: Vec<MoveRecord>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UndoReport {
    pub restored: usize,
    pub failed: Vec<String>,
    /// Moves that could not be restored; keep them for a later attempt.
    pub remaining: Vec<MoveRecord>,
}

/// Build a dry-run plan for `tracks`, rooted at `root`.
pub fn plan(tracks: &[Track], root: &Path, pattern: &str) -> Result<OrganizePlan> {
    let mut rows = Vec::with_capacity(tracks.len());
    let mut targets: Vec<(String, usize)> = Vec::new();

    for track in tracks {
        let current = PathBuf::from(&track.path);
        let mut row = OrganizeRow {
            track_id: track.id,
            title: track.title.clone(),
            current_path: current.clone(),
            target_path: current.clone(),
            status: MoveStatus::Ready,
            detail: String::new(),
        };
        if !current.is_file() {
            row.status = MoveStatus::Missing;
            row.detail = "source file is gone".to_string();
            rows.push(row);
            continue;
        }
        match target_for(root, track, pattern) {
            Ok(target) => row.target_path = target,
            Err(error) => {
                row.status = MoveStatus::Invalid;
                row.detail = format!("{error:#}");
                rows.push(row);
                continue;
            }
        }
        if row.target_path == current {
            row.status = MoveStatus::Unchanged;
        } else if row.target_path.exists() {
            row.status = MoveStatus::Collision;
            row.detail = "target already exists".to_string();
        }
        targets.push((path_key(&row.target_path), rows.len()));
        rows.push(row);
    }

    // Two tracks rendering to the same name collide even if nothing exists yet.
    let mut duplicate: Vec<bool> = vec![false; rows.len()];
    for (index, (key, row)) in targets.iter().enumerate() {
        if rows[*row].status != MoveStatus::Ready {
            continue;
        }
        if targets[..index].iter().any(|(other, _)| other == key) {
            duplicate[*row] = true;
        }
        if targets[index + 1..].iter().any(|(other, _)| other == key) {
            duplicate[*row] = true;
        }
    }
    for (row, duplicate) in rows.iter_mut().zip(duplicate) {
        if duplicate {
            row.status = MoveStatus::Collision;
            row.detail = "another track uses the same target".to_string();
        }
    }

    let ready = rows
        .iter()
        .filter(|row| row.status == MoveStatus::Ready)
        .count();
    Ok(OrganizePlan {
        skipped: rows.len() - ready,
        ready,
        rows,
    })
}

/// Render `pattern` for one track below `root`.
pub fn target_for(root: &Path, track: &Track, pattern: &str) -> Result<PathBuf> {
    let pattern = pattern.trim();
    anyhow::ensure!(!pattern.is_empty(), "the pattern is empty");
    anyhow::ensure!(
        !pattern.starts_with('/') && !pattern.starts_with('\\'),
        "the pattern must be relative to the music folder"
    );

    let source = Path::new(&track.path);
    let has_ext = pattern.contains("{ext}");
    let mut segments: Vec<String> = Vec::new();
    for raw in pattern.split('/') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let expanded = expand(raw, track, source)?;
        let segment = sanitize(&expanded);
        anyhow::ensure!(
            segment != "." && segment != "..",
            "the pattern cannot leave the music folder"
        );
        segments.push(segment);
    }
    anyhow::ensure!(!segments.is_empty(), "the pattern produced no filename");

    if !has_ext {
        if let Some(extension) = source.extension().and_then(|ext| ext.to_str()) {
            if !extension.is_empty() {
                let last = segments.last_mut().expect("segments is not empty");
                last.push('.');
                last.push_str(extension);
            }
        }
    }

    let mut target = root.to_path_buf();
    for segment in &segments {
        target.push(segment);
    }
    Ok(target)
}

/// Apply the ready rows. Unsafe or failing rows are reported, not forced.
pub fn apply(conn: &Connection, rows: &[OrganizeRow]) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    for row in rows.iter().filter(|row| row.status == MoveStatus::Ready) {
        let from = &row.current_path;
        let to = &row.target_path;
        if !from.is_file() {
            report
                .failed
                .push(format!("{}: source file is gone", from.display()));
            continue;
        }
        if to.exists() {
            report
                .failed
                .push(format!("{}: target already exists", to.display()));
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        if let Err(error) = std::fs::rename(from, to) {
            report.failed.push(format!("{}: {error}", from.display()));
            continue;
        }
        let changed = conn.execute(
            "UPDATE tracks SET path = ?1 WHERE id = ?2",
            params![to.to_string_lossy(), row.track_id],
        );
        match changed {
            Ok(1) => {
                report.records.push(MoveRecord {
                    track_id: row.track_id,
                    from: from.clone(),
                    to: to.clone(),
                });
                report.moved += 1;
            }
            other => {
                let _ = std::fs::rename(to, from);
                let reason = match other {
                    Ok(_) => "library row disappeared".to_string(),
                    Err(error) => error.to_string(),
                };
                report.failed.push(format!("{}: {reason}", from.display()));
            }
        }
    }
    Ok(report)
}

/// Move the recorded files back and restore their library rows.
pub fn undo(conn: &Connection, records: &[MoveRecord]) -> Result<UndoReport> {
    let mut report = UndoReport::default();
    for record in records.iter().rev() {
        let from = &record.from;
        let to = &record.to;
        if !to.is_file() {
            report
                .failed
                .push(format!("{}: moved file is gone", to.display()));
            report.remaining.push(record.clone());
            continue;
        }
        if from.exists() {
            report
                .failed
                .push(format!("{}: original path is occupied", from.display()));
            report.remaining.push(record.clone());
            continue;
        }
        if let Some(parent) = from.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        if let Err(error) = std::fs::rename(to, from) {
            report.failed.push(format!("{}: {error}", to.display()));
            report.remaining.push(record.clone());
            continue;
        }
        let changed = conn.execute(
            "UPDATE tracks SET path = ?1 WHERE id = ?2 AND path = ?3",
            params![
                from.to_string_lossy(),
                record.track_id,
                to.to_string_lossy()
            ],
        );
        if matches!(changed, Ok(1)) {
            report.restored += 1;
        } else {
            let _ = std::fs::rename(from, to);
            report.failed.push(format!(
                "{}: library row changed since the move",
                from.display()
            ));
            report.remaining.push(record.clone());
        }
    }
    report.remaining.reverse();
    Ok(report)
}

// ---- journal ----

/// Persist the last move batch atomically.
pub fn save_undo(path: &Path, records: &[MoveRecord]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec(records)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_undo(path: &Path) -> Result<Option<Vec<MoveRecord>>> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn clear_undo(path: &Path) {
    let _ = std::fs::remove_file(path);
}

// ---- pattern rendering ----

fn expand(segment: &str, track: &Track, source: &Path) -> Result<String> {
    let mut out = String::new();
    let mut rest = segment;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            anyhow::bail!("unclosed placeholder in the pattern");
        };
        out.push_str(&token(&rest[start + 1..start + end], track, source)?);
        rest = &rest[start + end + 1..];
    }
    anyhow::ensure!(!rest.contains('}'), "unexpected '}}' in the pattern");
    out.push_str(rest);
    Ok(out)
}

fn token(spec: &str, track: &Track, source: &Path) -> Result<String> {
    let (key, width) = match spec.split_once(':') {
        Some((key, width)) => (key.trim(), width.trim().parse::<usize>().ok()),
        None => (spec.trim(), None),
    };
    let key = key.to_ascii_lowercase();
    let (value, numeric) = match key.as_str() {
        "artist" => (non_empty(&track.artist, "Unknown Artist"), false),
        "album" => (non_empty(&track.album, "Unknown Album"), false),
        "album_artist" | "albumartist" => {
            let value = non_empty(&track.album_artist, "");
            if value.is_empty() {
                (non_empty(&track.artist, "Unknown Artist"), false)
            } else {
                (value, false)
            }
        }
        "title" => (non_empty(&track.title, &file_stem(source)), false),
        "genre" => (non_empty(&track.genre, "Unknown Genre"), false),
        "track" => (
            track.track_no.map(|n| n.to_string()).unwrap_or_default(),
            true,
        ),
        "disc" => (
            track.disc_no.map(|n| n.to_string()).unwrap_or_default(),
            true,
        ),
        "year" => (track.year.map(|n| n.to_string()).unwrap_or_default(), true),
        "ext" => (
            source
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_string(),
            false,
        ),
        _ => anyhow::bail!("unknown placeholder {{{spec}}}"),
    };
    if numeric {
        let width = width.unwrap_or(2);
        if value.is_empty() {
            return Ok("0".repeat(width));
        }
        return Ok(format!("{value:0>width$}"));
    }
    Ok(value)
}

fn non_empty(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Unknown Title".to_string())
}

/// Replace characters that are invalid in a path component on any platform.
fn sanitize(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() {
        return "Unknown".to_string();
    }
    if is_reserved(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}
