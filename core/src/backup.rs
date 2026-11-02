//! Library database backups.
//!
//! Backups are plain SQLite files written with `VACUUM INTO`, so they include
//! everything committed to the WAL and need no sidecar files. Restoring is
//! handled by [`crate::Library::restore_from`], which writes a safety backup of
//! the current database first.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct BackupFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Directory holding timestamped library backups.
pub fn dir() -> Result<PathBuf> {
    let dir = crate::db::data_dir()?.join("backups");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Backups, newest first.
pub fn list() -> Result<Vec<BackupFile>> {
    let dir = dir()?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("db") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            files.push(BackupFile {
                path,
                bytes: meta.len(),
            });
        }
    }
    files.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
    Ok(files)
}

/// Write a consistent, compact backup of `conn` into the backup directory.
pub fn create(conn: &Connection) -> Result<PathBuf> {
    let dir = dir()?;
    let mut path = dir.join(format!("library-{}.db", timestamp()));
    let mut index = 2;
    while path.exists() {
        path = dir.join(format!("library-{}-{index}.db", timestamp()));
        index += 1;
    }
    create_at(conn, &path)?;
    Ok(path)
}

/// Write a backup to an explicit path. The target must not exist.
pub fn create_at(conn: &Connection, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    conn.execute("VACUUM INTO ?1", rusqlite::params![path.to_string_lossy()])
        .with_context(|| format!("cannot write backup {}", path.display()))?;
    Ok(())
}

/// Check that `path` is a readable SQLite database with a tracks table.
pub fn validate(path: &Path) -> Result<()> {
    anyhow::ensure!(path.is_file(), "backup file not found: {}", path.display());
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open backup {}", path.display()))?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(check == "ok", "backup failed its integrity check");
    let tracks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'tracks'",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(tracks == 1, "that file is not a MusicWasp library");
    Ok(())
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}-{:02}{:02}{:02}",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// UTC civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_726), (2026, 9, 30));
    }
}
