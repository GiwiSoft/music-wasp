//! Reconciliation between vanished library rows and files still on disk.
//!
//! [`reconcile`] matches rows whose indexed path no longer exists against audio
//! files that are present. It never writes: callers show the returned preview
//! and pass the accepted candidates to [`apply`], which keeps each row and its
//! ratings/history while following the file to its new path.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::library::{
    audio_summary, file_mtime, is_audio_path, list, normalize_tag, refresh_file, Track,
};

/// Minimum combined score before a file is offered as a rename candidate.
const MIN_SCORE: i64 = 6;
/// Independent file signals required to merge away an already indexed row.
const MERGE_FILE_SIGNALS: usize = 2;

/// A vanished library row matched to an audio file that still exists on disk.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileCandidate {
    /// The indexed row that lost its file. Ratings and play history live here.
    pub track: Track,
    pub new_path: String,
    /// Row that already owns `new_path`, when the watcher re-imported the
    /// renamed file before reconciliation ran. Applying the move merges it away.
    pub replaces_id: Option<i64>,
    pub score: i64,
    pub reasons: Vec<String>,
}

/// Read-only preview of what a reconciliation scan would do.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcilePlan {
    /// Every row whose indexed file is gone.
    pub missing: Vec<Track>,
    /// Missing rows with no convincing file match; pruning them stays manual.
    pub unmatched: Vec<Track>,
    pub candidates: Vec<ReconcileCandidate>,
    pub scanned_files: usize,
}

/// Result of applying a reconciliation preview.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileOutcome {
    pub moved: usize,
    /// Duplicate rows removed because the watcher had re-imported the file.
    pub removed: Vec<i64>,
}

/// Match vanished rows against files under `roots`. Never writes.
///
/// Files that are already indexed become merge candidates and need two
/// independent file signals, so unrelated tracks that only share tags are not
/// merged by accident. Unindexed files can also match on tags alone, with a
/// minimum score.
pub fn reconcile(conn: &Connection, roots: &[PathBuf]) -> Result<ReconcilePlan> {
    let tracks = list(conn, None)?;
    let meta = stored_file_meta(conn)?;
    let missing: Vec<(Track, Facts)> = tracks
        .iter()
        .filter(|track| !Path::new(&track.path).is_file())
        .map(|track| {
            let file_meta = meta.get(&track.id).copied().unwrap_or_default();
            (track.clone(), Facts::from_track(track, file_meta))
        })
        .collect();

    let roots: Vec<PathBuf> = roots.iter().filter(|root| root.is_dir()).cloned().collect();
    if missing.is_empty() || roots.is_empty() {
        return Ok(plan(missing, Vec::new(), 0));
    }

    let by_id: HashMap<i64, &Track> = tracks.iter().map(|track| (track.id, track)).collect();
    let by_path: HashMap<PathBuf, i64> = tracks
        .iter()
        .map(|track| (PathBuf::from(&track.path), track.id))
        .collect();
    let index = FileIndex::scan(&roots, &by_path, &by_id, &meta);

    let mut pairs = Vec::new();
    for (missing_index, (track, facts)) in missing.iter().enumerate() {
        for file_index in index.candidates(facts) {
            let file = &index.files[file_index];
            if file.owner == Some(track.id) {
                continue;
            }
            let signals = Signals::between(facts, &file.facts);
            if signals.accepts(file.owner.is_some()) {
                pairs.push(Pair {
                    score: signals.score(),
                    unindexed: file.owner.is_none(),
                    missing: missing_index,
                    file: file_index,
                });
            }
        }
    }
    pairs.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.unindexed.cmp(&a.unindexed))
            .then_with(|| a.missing.cmp(&b.missing))
            .then_with(|| index.files[a.file].path.cmp(&index.files[b.file].path))
    });

    let candidates = assign(pairs, &index, &missing);
    Ok(plan(missing, candidates, index.files.len()))
}

/// Assemble the final preview: greedy one-to-one assignment, sorted by score.
fn assign(
    pairs: Vec<Pair>,
    index: &FileIndex,
    missing: &[(Track, Facts)],
) -> Vec<ReconcileCandidate> {
    let mut used_missing = vec![false; missing.len()];
    let mut used_files = vec![false; index.files.len()];
    let mut candidates = Vec::new();
    for pair in pairs {
        if used_missing[pair.missing] || used_files[pair.file] {
            continue;
        }
        used_missing[pair.missing] = true;
        used_files[pair.file] = true;

        let (track, facts) = &missing[pair.missing];
        let file = &index.files[pair.file];
        candidates.push(ReconcileCandidate {
            track: track.clone(),
            new_path: file.path.to_string_lossy().into_owned(),
            replaces_id: file.owner,
            score: pair.score,
            reasons: Signals::between(facts, &file.facts).reasons(),
        });
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.score));
    candidates
}

fn plan(
    missing: Vec<(Track, Facts)>,
    candidates: Vec<ReconcileCandidate>,
    scanned_files: usize,
) -> ReconcilePlan {
    let matched: HashSet<i64> = candidates.iter().map(|c| c.track.id).collect();
    let missing: Vec<Track> = missing.into_iter().map(|(track, _)| track).collect();
    ReconcilePlan {
        unmatched: missing
            .iter()
            .filter(|track| !matched.contains(&track.id))
            .cloned()
            .collect(),
        missing,
        candidates,
        scanned_files,
    }
}

/// Apply a reviewed preview. Rows keep their identity, ratings and history;
/// only the path changes, then the file is re-read so tag edits ride along.
pub fn apply(conn: &Connection, candidates: &[ReconcileCandidate]) -> Result<ReconcileOutcome> {
    let mut outcome = ReconcileOutcome::default();
    if candidates.is_empty() {
        return Ok(outcome);
    }
    let tx = conn.unchecked_transaction()?;
    for candidate in candidates {
        if let Some(removed) = apply_one(&tx, candidate)? {
            outcome.removed.push(removed);
        }
        outcome.moved += 1;
    }
    tx.commit()?;
    Ok(outcome)
}

fn apply_one(conn: &Connection, candidate: &ReconcileCandidate) -> Result<Option<i64>> {
    let path = Path::new(&candidate.new_path);
    anyhow::ensure!(path.is_file(), "file is gone: {}", candidate.new_path);
    let meta = std::fs::metadata(path)?;
    let size = meta.len() as i64;
    let mtime = file_mtime(&meta);

    let occupant: Option<(i64, i64, i64)> = conn
        .query_row(
            "SELECT id, size_bytes, mtime FROM tracks WHERE path = ?1 AND id <> ?2",
            params![candidate.new_path, candidate.track.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let removed = match occupant {
        Some((id, stored_size, stored_mtime)) => {
            anyhow::ensure!(
                stored_size == size && stored_mtime == mtime,
                "another library entry already points at {}; rescan before applying",
                candidate.new_path
            );
            conn.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
            Some(id)
        }
        None => None,
    };

    let changed = conn.execute(
        "UPDATE tracks SET path = ?1 WHERE id = ?2",
        params![candidate.new_path, candidate.track.id],
    )?;
    anyhow::ensure!(changed == 1, "track row disappeared; rescan");
    refresh_file(conn, path)?;
    Ok(removed)
}

fn stored_file_meta(conn: &Connection) -> Result<HashMap<i64, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT id, size_bytes, mtime FROM tracks")?;
    let mut rows = stmt.query([])?;
    let mut meta = HashMap::new();
    while let Some(row) = rows.next()? {
        meta.insert(row.get(0)?, (row.get(1)?, row.get(2)?));
    }
    Ok(meta)
}

/// The tag and file attributes used to compare a row with a candidate file.
struct Facts {
    size: i64,
    mtime: i64,
    duration: u64,
    title: String,
    artist: String,
    album: String,
    format: String,
}

impl Facts {
    fn from_track(track: &Track, (size, mtime): (i64, i64)) -> Self {
        Self {
            size,
            mtime,
            duration: track.duration_secs,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            format: track.format.clone(),
        }
    }

    fn from_file(path: &Path, meta: &std::fs::Metadata) -> Option<Self> {
        let info = audio_summary(path)?;
        Some(Self {
            size: meta.len() as i64,
            mtime: file_mtime(meta),
            duration: info.duration_secs,
            title: info.title,
            artist: info.artist,
            album: info.album,
            format: info.format,
        })
    }

    /// Normalized key used to bucket files with the same tags.
    fn tag_key(&self) -> Option<(String, String, String)> {
        let title = normalize_tag(&self.title);
        if title.is_empty() {
            return None;
        }
        Some((
            title,
            normalize_tag(&self.artist),
            normalize_tag(&self.album),
        ))
    }
}

/// Which signals matched between a missing row and a candidate file.
#[derive(Clone, Copy, Default)]
struct Signals {
    size: bool,
    mtime: bool,
    duration: bool,
    title: bool,
    artist: bool,
    album: bool,
    format: bool,
}

impl Signals {
    fn between(old: &Facts, new: &Facts) -> Self {
        let same_tag = |a: &str, b: &str| {
            let a = normalize_tag(a);
            !a.is_empty() && a == normalize_tag(b)
        };
        Self {
            size: old.size > 0 && old.size == new.size,
            mtime: old.mtime > 0 && old.mtime == new.mtime,
            duration: old.duration > 0 && old.duration == new.duration,
            title: same_tag(&old.title, &new.title),
            artist: same_tag(&old.artist, &new.artist),
            album: same_tag(&old.album, &new.album),
            format: !old.format.is_empty() && old.format.eq_ignore_ascii_case(&new.format),
        }
    }

    fn score(self) -> i64 {
        weighted([
            (self.size, 4),
            (self.mtime, 4),
            (self.duration, 2),
            (self.title, 3),
            (self.artist, 2),
            (self.album, 1),
            (self.format, 1),
        ])
    }

    fn file_signals(self) -> usize {
        [self.size, self.mtime, self.duration]
            .into_iter()
            .filter(|matched| *matched)
            .count()
    }

    fn tag_pair(self) -> bool {
        self.title && self.artist
    }

    fn accepts(self, already_indexed: bool) -> bool {
        let likely = self.score() >= MIN_SCORE && (self.file_signals() > 0 || self.tag_pair());
        likely && (!already_indexed || self.file_signals() >= MERGE_FILE_SIGNALS)
    }

    fn reasons(self) -> Vec<String> {
        [
            (self.size, "size"),
            (self.mtime, "mtime"),
            (self.duration, "duration"),
            (self.title, "title"),
            (self.artist, "artist"),
            (self.album, "album"),
            (self.format, "format"),
        ]
        .into_iter()
        .filter(|(matched, _)| *matched)
        .map(|(_, label)| label.to_string())
        .collect()
    }
}

struct DiskFile {
    path: PathBuf,
    owner: Option<i64>,
    facts: Facts,
}

impl DiskFile {
    /// Indexed files reuse the stored tags; new files are read once.
    fn load(
        path: PathBuf,
        owner: Option<i64>,
        by_id: &HashMap<i64, &Track>,
        meta: &HashMap<i64, (i64, i64)>,
    ) -> Option<Self> {
        let direct = std::fs::metadata(&path).ok()?;
        let facts = match owner.and_then(|id| by_id.get(&id).copied()) {
            Some(track) => {
                Facts::from_track(track, meta.get(&track.id).copied().unwrap_or_default())
            }
            None => Facts::from_file(&path, &direct)?,
        };
        Some(Self { path, owner, facts })
    }
}

/// Candidate files, bucketed by the cheap signals so each missing row only
/// scores a handful of neighbours instead of the whole library.
#[derive(Default)]
struct FileIndex {
    files: Vec<DiskFile>,
    by_meta: HashMap<(i64, i64), Vec<usize>>,
    by_size: HashMap<i64, Vec<usize>>,
    by_duration: HashMap<u64, Vec<usize>>,
    by_tags: HashMap<(String, String, String), Vec<usize>>,
}

impl FileIndex {
    fn scan(
        roots: &[PathBuf],
        by_path: &HashMap<PathBuf, i64>,
        by_id: &HashMap<i64, &Track>,
        meta: &HashMap<i64, (i64, i64)>,
    ) -> Self {
        let mut index = Self::default();
        let mut seen = HashSet::new();
        for root in roots {
            for entry in WalkDir::new(root).follow_links(true).into_iter().flatten() {
                let path = entry.path();
                if !entry.file_type().is_file() || !is_audio_path(path) {
                    continue;
                }
                if !seen.insert(path.to_path_buf()) {
                    continue;
                }
                let owner = by_path.get(path).copied();
                if let Some(file) = DiskFile::load(path.to_path_buf(), owner, by_id, meta) {
                    index.push(file);
                }
            }
        }
        index
    }

    fn push(&mut self, file: DiskFile) {
        let index = self.files.len();
        let facts = &file.facts;
        if facts.size > 0 {
            self.by_size.entry(facts.size).or_default().push(index);
            if facts.mtime > 0 {
                self.by_meta
                    .entry((facts.size, facts.mtime))
                    .or_default()
                    .push(index);
            }
        }
        if facts.duration > 0 {
            self.by_duration
                .entry(facts.duration)
                .or_default()
                .push(index);
        }
        if let Some(key) = facts.tag_key() {
            self.by_tags.entry(key).or_default().push(index);
        }
        self.files.push(file);
    }

    fn candidates(&self, facts: &Facts) -> Vec<usize> {
        let mut indexes = Vec::new();
        extend(&mut indexes, self.by_meta.get(&(facts.size, facts.mtime)));
        extend(&mut indexes, self.by_size.get(&facts.size));
        extend(&mut indexes, self.by_duration.get(&facts.duration));
        if let Some(key) = facts.tag_key() {
            extend(&mut indexes, self.by_tags.get(&key));
        }
        indexes.sort_unstable();
        indexes.dedup();
        indexes
    }
}

fn weighted(signals: [(bool, i64); 7]) -> i64 {
    signals
        .into_iter()
        .filter(|(matched, _)| *matched)
        .map(|(_, weight)| weight)
        .sum()
}

fn extend(dest: &mut Vec<usize>, source: Option<&Vec<usize>>) {
    if let Some(source) = source {
        dest.extend_from_slice(source);
    }
}

struct Pair {
    score: i64,
    unindexed: bool,
    missing: usize,
    file: usize,
}
