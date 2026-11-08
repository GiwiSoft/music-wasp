//! Lyrics lookup and `.lrc` parsing.
//!
//! Sidecar files win over embedded tag lyrics. Both synchronized `.lrc` files
//! (with `[mm:ss.xx]` timestamps) and plain unsynchronized text are supported.

use lofty::file::TaggedFileExt;
use lofty::prelude::ItemKey;
use lofty::probe::Probe;
use std::path::{Path, PathBuf};

/// One timestamped or plain lyric line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

/// Parsed lyrics for one track.
#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// True when the lines carry timestamps and can follow playback.
    pub synced: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// `[offset:]` from the file, already applied to `lines`.
    pub offset_ms: i64,
    /// True when the text came from a tag instead of a sidecar file.
    pub embedded: bool,
}

impl Lyrics {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Index of the line that should be highlighted at `position_secs`.
    /// Returns `None` before the first timestamp or for unsynchronized lyrics.
    pub fn line_index(&self, position_secs: f64) -> Option<usize> {
        if !self.synced || self.lines.is_empty() || !position_secs.is_finite() {
            return None;
        }
        let position_ms = (position_secs.max(0.0) * 1000.0) as u64;
        if position_ms < self.lines[0].time_ms {
            return None;
        }
        self.lines
            .partition_point(|line| line.time_ms <= position_ms)
            .checked_sub(1)
    }
}

/// Load lyrics for an audio file: sidecar `.lrc` first, then tag lyrics.
pub fn load(audio: &Path) -> Option<Lyrics> {
    if let Some(path) = sidecar_path(audio) {
        if let Ok(bytes) = std::fs::read(&path) {
            let text = decode(&bytes);
            let lyrics = parse(&text);
            if !lyrics.is_empty() {
                return Some(lyrics);
            }
        }
    }
    embedded(audio).filter(|lyrics| !lyrics.is_empty())
}

/// The `.lrc` file beside `audio`, matched case-insensitively when needed.
pub fn sidecar_path(audio: &Path) -> Option<PathBuf> {
    let lower = audio.with_extension("lrc");
    if lower.is_file() {
        return Some(lower);
    }
    let upper = audio.with_extension("LRC");
    if upper.is_file() {
        return Some(upper);
    }
    let stem = audio.file_stem()?.to_string_lossy().to_lowercase();
    let directory = audio.parent()?;
    let entries = std::fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let matches_extension = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lrc"));
        let matches_stem = path
            .file_stem()
            .is_some_and(|candidate| candidate.to_string_lossy().to_lowercase() == stem);
        if matches_extension && matches_stem {
            return Some(path);
        }
    }
    None
}

fn embedded(audio: &Path) -> Option<Lyrics> {
    let tagged = Probe::open(audio).ok()?.read().ok()?;
    let text = tagged
        .tags()
        .iter()
        .filter_map(|tag| tag.get_string(&ItemKey::Lyrics))
        .find(|value| !value.trim().is_empty())?;
    let mut lyrics = parse(text);
    lyrics.embedded = true;
    Some(lyrics)
}

/// Parse `.lrc` text. Untimed text becomes an unsynchronized lyric list.
pub fn parse(text: &str) -> Lyrics {
    let mut lyrics = Lyrics::default();
    let mut unsynced = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }

        let mut rest = line;
        let mut times = Vec::new();
        let mut is_metadata = false;

        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else {
                break;
            };
            let inner = &stripped[..close];
            if let Some(time_ms) = timestamp_ms(inner) {
                times.push(time_ms);
                rest = &stripped[close + 1..];
                continue;
            }
            if times.is_empty() {
                if let Some((key, value)) = metadata(inner) {
                    apply_metadata(&mut lyrics, key, value);
                    is_metadata = true;
                    rest = &stripped[close + 1..];
                    continue;
                }
            }
            break;
        }

        if times.is_empty() {
            if !is_metadata && !rest.trim().is_empty() {
                let line = strip_word_timestamps(rest).trim().to_string();
                if !line.is_empty() {
                    unsynced.push(line);
                }
            }
            continue;
        }

        let line = strip_word_timestamps(rest).trim().to_string();
        if line.is_empty() {
            // Empty timestamps mark instrumental gaps; keep the gap out of the
            // visible list instead of rendering blank rows.
            continue;
        }
        for time_ms in times {
            lyrics.lines.push(LyricLine {
                time_ms,
                text: line.clone(),
            });
        }
    }

    if lyrics.lines.is_empty() {
        lyrics.lines = unsynced
            .into_iter()
            .map(|text| LyricLine { time_ms: 0, text })
            .collect();
        lyrics.synced = false;
    } else {
        lyrics.synced = true;
        if lyrics.offset_ms != 0 {
            for line in &mut lyrics.lines {
                line.time_ms = line.time_ms.saturating_add_signed(lyrics.offset_ms);
            }
        }
        lyrics.lines.sort_by_key(|line| line.time_ms);
    }
    lyrics
}

fn metadata(value: &str) -> Option<(&str, &str)> {
    let (key, value) = value.split_once(':')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, value))
}

fn apply_metadata(lyrics: &mut Lyrics, key: &str, value: &str) {
    let value = value.trim();
    match key.to_ascii_lowercase().as_str() {
        "ti" => lyrics.title = non_empty(value),
        "ar" => lyrics.artist = non_empty(value),
        "al" => lyrics.album = non_empty(value),
        "offset" => {
            if let Ok(offset) = value.parse::<i64>() {
                lyrics.offset_ms = offset;
            }
        }
        // `by`, `re`, `ve`, `length` and unknown keys carry no display state.
        _ => {}
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse `mm:ss`, `mm:ss.xx` and the older `mm:ss:xx` forms into milliseconds.
fn timestamp_ms(value: &str) -> Option<u64> {
    let parts: Vec<&str> = value.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let minutes: u64 = parts[0].trim().parse().ok()?;
    let seconds_text = parts[1].trim();
    let seconds: f64 = seconds_text.parse().ok()?;
    if seconds < 0.0 {
        return None;
    }
    let mut fraction_ms = (seconds.fract() * 1000.0).round() as u64;
    if parts.len() == 3 {
        if seconds_text.contains('.') {
            return None;
        }
        let raw = parts[2].trim();
        if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let digits = raw.len().min(3);
        let value: u64 = raw[..digits].parse().ok()?;
        fraction_ms = match digits {
            1 => value * 100,
            2 => value * 10,
            _ => value,
        };
    }
    Some(minutes * 60_000 + seconds.trunc() as u64 * 1000 + fraction_ms)
}

/// Remove enhanced-LRC word timestamps such as `<00:12.34>` from a line.
fn strip_word_timestamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('>') {
            if timestamp_ms(&after[..close]).is_some() {
                rest = &after[close + 1..];
                continue;
            }
        }
        out.push('<');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

fn utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lrc() {
        let text = "\u{feff}[ti:Demo]\r\n\
            [ar:Artist]\r\n\
            [al:Album]\r\n\
            [offset:+250]\r\n\
            [00:01.50]First line\r\n\
            [00:03.25][00:10.00]Repeated\r\n\
            [1:05]Late line\r\n";
        let lyrics = parse(text);
        assert!(lyrics.synced);
        assert_eq!(lyrics.title.as_deref(), Some("Demo"));
        assert_eq!(lyrics.artist.as_deref(), Some("Artist"));
        assert_eq!(lyrics.album.as_deref(), Some("Album"));
        assert_eq!(lyrics.offset_ms, 250);
        assert_eq!(lyrics.lines.len(), 4);
        assert_eq!(lyrics.lines[0].time_ms, 1750);
        assert_eq!(lyrics.lines[0].text, "First line");
        assert_eq!(lyrics.lines[1].time_ms, 3500);
        assert_eq!(lyrics.lines[2].time_ms, 10_250);
        assert_eq!(lyrics.lines[3].time_ms, 65_250);
        assert_eq!(lyrics.lines[3].text, "Late line");
    }

    #[test]
    fn parses_colon_fractions_and_word_times() {
        let lyrics = parse("[00:12:34]<00:12.34>Hello <00:13.00>world");
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].time_ms, 12_340);
        assert_eq!(lyrics.lines[0].text, "Hello world");
    }

    #[test]
    fn accepts_unnormalized_seconds() {
        let lyrics = parse("[00:75.00]Rolled over");
        assert_eq!(lyrics.lines[0].time_ms, 75_000);
    }

    #[test]
    fn parses_plain_lyrics() {
        let lyrics = parse("First line\n\nSecond line\n");
        assert!(!lyrics.synced);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "First line");
        assert_eq!(lyrics.lines[1].text, "Second line");
    }

    #[test]
    fn keeps_section_markers_as_plain_lyrics() {
        let lyrics = parse("[Verse 1]\nSinging");
        assert!(!lyrics.synced);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "[Verse 1]");
    }

    #[test]
    fn active_line_follows_position() {
        let lyrics = parse("[00:01.00]One\n[00:05.00]Two\n[00:09.00]Three");
        assert_eq!(lyrics.line_index(0.5), None);
        assert_eq!(lyrics.line_index(1.0), Some(0));
        assert_eq!(lyrics.line_index(4.99), Some(0));
        assert_eq!(lyrics.line_index(5.0), Some(1));
        assert_eq!(lyrics.line_index(60.0), Some(2));
    }

    #[test]
    fn unsynced_lyrics_have_no_active_line() {
        let lyrics = parse("Just text");
        assert_eq!(lyrics.line_index(10.0), None);
    }
}
