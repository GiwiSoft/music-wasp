//! Auto-tag by album: look a release up on MusicBrainz, match its tracks
//! against local files, then write the metadata and the front cover into them.
//!
//! MusicBrainz asks for one request per second and a real User-Agent; both are
//! enforced here. Cover art comes from the Cover Art Archive, which needs no
//! key. AcoustID identification additionally needs an application API key and
//! runs the pure-Rust Chromaprint port; it is optional and only runs when
//! requested.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const USER_AGENT: &str = concat!(
    "MusicWasp/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/xavier/music-wasp )"
);

const MB_API: &str = "https://musicbrainz.org/ws/2";
const CAA_API: &str = "https://coverartarchive.org";
const ACOUSTID_API: &str = "https://api.acoustid.org/v2/lookup";

fn mb_api() -> String {
    std::env::var("MUSICWASP_MUSICBRAINZ_API").unwrap_or_else(|_| MB_API.to_string())
}

fn caa_api() -> String {
    std::env::var("MUSICWASP_COVERART_API").unwrap_or_else(|_| CAA_API.to_string())
}

fn acoustid_api() -> String {
    std::env::var("MUSICWASP_ACOUSTID_API").unwrap_or_else(|_| ACOUSTID_API.to_string())
}

/// Application key compiled in from the build environment or the local `.env`.
/// Empty when the build had neither.
pub fn builtin_acoustid_key() -> &'static str {
    option_env!("MUSICWASP_ACOUSTID_KEY").unwrap_or("")
}

/// Key used at runtime: the key baked in at build time. Debug builds accept a
/// `MUSICWASP_ACOUSTID_KEY` override for development and tests; release builds
/// have no runtime override.
pub fn acoustid_key() -> String {
    #[cfg(debug_assertions)]
    {
        let runtime = std::env::var("MUSICWASP_ACOUSTID_KEY").unwrap_or_default();
        if !runtime.trim().is_empty() {
            return runtime.trim().to_string();
        }
    }
    builtin_acoustid_key().to_string()
}

// ---------------------------------------------------------------- public types

/// A release returned by search, before we fetch its tracks.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub release_id: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub track_count: usize,
    /// MusicBrainz relevance score, 0-100.
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseTrack {
    pub disc: u32,
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub length_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub tracks: Vec<ReleaseTrack>,
    pub has_front_cover: bool,
}

/// The subset of a local track the matcher needs.
#[derive(Debug, Clone)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub disc_no: Option<i32>,
    pub track_no: Option<i32>,
    pub title: String,
    pub duration_secs: u64,
}

/// One local file paired with the release track it should become.
#[derive(Debug, Clone)]
pub struct Match {
    pub path: PathBuf,
    pub track: ReleaseTrack,
}

/// Bytes of a downloaded front cover and its MIME type.
pub struct Cover {
    pub data: Vec<u8>,
    pub mime: String,
}

/// Chromaprint output for one file.
#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint {
    pub duration: u64,
    pub fingerprint: String,
}

// ---------------------------------------------------------------- HTTP plumbing

/// One client shares the connection pool across requests. Cloning it is cheap.
fn client() -> Result<reqwest::blocking::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let built = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .context("cannot build HTTP client")?;
    Ok(CLIENT.get_or_init(|| built).clone())
}

/// Largest cover image accepted from the Cover Art Archive.
const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

/// MusicBrainz allows one request per second per client. Local fake servers in
/// tests do not need the delay.
fn throttle(url: &str) {
    if !url.starts_with("https://musicbrainz.org") {
        return;
    }
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < Duration::from_millis(1_050) {
            std::thread::sleep(Duration::from_millis(1_050) - elapsed);
        }
    }
    *last = Some(Instant::now());
}

/// GET a JSON document, retrying the transient failures MusicBrainz throws at
/// clients that talk a little too fast (429/503), honouring `Retry-After`.
fn get_json<T: DeserializeOwned>(url: &str, query: &[(&str, &str)], what: &str) -> Result<T> {
    const ATTEMPTS: u32 = 4;
    let mut last_error = None;

    for attempt in 1..=ATTEMPTS {
        throttle(url);
        let response = match client()?.get(url).query(query).send() {
            Ok(r) => r,
            Err(e) => {
                last_error = Some(anyhow::anyhow!("{what}: {e}"));
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }
        };

        let status = response.status();
        if status.is_success() {
            return response
                .json::<T>()
                .with_context(|| format!("cannot parse {what} reply"));
        }

        let retryable = status.as_u16() == 429 || status.is_server_error();
        let wait = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(attempt as u64 * 2);
        last_error = Some(anyhow::anyhow!("{what} returned {status}"));

        if !retryable || attempt == ATTEMPTS {
            break;
        }
        std::thread::sleep(Duration::from_secs(wait.min(10)));
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("{what} failed")))
}

// ---------------------------------------------------------------- search

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    releases: Vec<SearchRelease>,
}

#[derive(Deserialize)]
struct SearchRelease {
    id: String,
    #[serde(default)]
    score: u32,
    #[serde(default)]
    title: String,
    date: Option<String>,
    country: Option<String>,
    #[serde(rename = "track-count", default)]
    track_count: Option<usize>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Deserialize)]
struct ArtistCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: String,
}

fn credit_to_string(credits: &[ArtistCredit]) -> String {
    credits
        .iter()
        .map(|c| format!("{}{}", c.name, c.joinphrase))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Search releases for an artist + album name.
pub fn search(artist: &str, album: &str) -> Result<Vec<Candidate>> {
    let query = format!(
        "release:\"{}\" AND artist:\"{}\"",
        escape(album),
        escape(artist)
    );

    let api = mb_api();
    let parsed: SearchResponse = get_json(
        &format!("{api}/release"),
        &[("query", query.as_str()), ("fmt", "json"), ("limit", "12")],
        "MusicBrainz search",
    )?;

    Ok(parsed
        .releases
        .into_iter()
        .map(|r| Candidate {
            artist: credit_to_string(&r.artist_credit),
            release_id: r.id,
            title: r.title,
            date: r.date,
            country: r.country,
            track_count: r.track_count.unwrap_or(0),
            score: r.score,
        })
        .collect())
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------- AcoustID

/// Compute a Chromaprint fingerprint with the pure-Rust decoder and
/// fingerprinter. No external `fpcalc` or codec library is required.
#[cfg(feature = "audio")]
pub fn fingerprint(path: &Path) -> Result<Fingerprint> {
    use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};
    use rodio::Source;

    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))
        .with_context(|| format!("cannot decode {}", path.display()))?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    if sample_rate == 0 || channels == 0 {
        anyhow::bail!("{} has no audio stream", path.display());
    }

    let configuration = rusty_chromaprint::Configuration::default();
    let mut printer = rusty_chromaprint::Fingerprinter::new(&configuration);
    printer
        .start(sample_rate, u32::from(channels))
        .map_err(|error| anyhow::anyhow!("cannot resample {}: {error}", path.display()))?;

    let mut samples = 0u64;
    let mut buffer = Vec::with_capacity(8192);
    for sample in decoder {
        buffer.push(sample);
        samples += 1;
        if buffer.len() >= 8192 {
            printer.consume(&buffer);
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        printer.consume(&buffer);
    }
    printer.finish();

    let raw = printer.fingerprint();
    if raw.is_empty() {
        anyhow::bail!("could not compute a fingerprint for {}", path.display());
    }
    let compressed = rusty_chromaprint::FingerprintCompressor::from(&configuration).compress(raw);
    let fingerprint = BASE64_URL_SAFE_NO_PAD.encode(compressed);
    let duration = (samples as f64 / (f64::from(sample_rate) * f64::from(channels)))
        .round()
        .max(1.0) as u64;

    Ok(Fingerprint {
        duration,
        fingerprint,
    })
}

/// Fingerprinting needs the audio decoder, which is part of the default build.
#[cfg(not(feature = "audio"))]
pub fn fingerprint(_path: &Path) -> Result<Fingerprint> {
    anyhow::bail!("this build has no audio decoder for fingerprinting")
}

#[derive(Deserialize)]
struct AcoustIdResponse {
    status: Option<String>,
    error: Option<AcoustIdError>,
    #[serde(default)]
    results: Vec<AcoustIdResult>,
}

#[derive(Deserialize)]
struct AcoustIdError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct AcoustIdResult {
    #[serde(default)]
    score: f32,
    #[serde(default)]
    recordings: Vec<AcoustIdRecording>,
}

#[derive(Deserialize)]
struct AcoustIdRecording {
    #[serde(default)]
    title: String,
    #[serde(default)]
    artists: Vec<AcoustIdArtist>,
    #[serde(default)]
    releases: Vec<AcoustIdRelease>,
}

#[derive(Deserialize)]
struct AcoustIdArtist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: String,
}

#[derive(Deserialize)]
struct AcoustIdRelease {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: Option<AcoustIdDate>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    track_count: Option<usize>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AcoustIdDate {
    Object {
        #[serde(default)]
        year: Option<i32>,
    },
    Text(String),
}

impl AcoustIdDate {
    fn year(&self) -> Option<i32> {
        match self {
            Self::Object { year } => *year,
            Self::Text(text) => text.get(0..4)?.parse().ok(),
        }
    }
}

/// Look up a Chromaprint fingerprint. Requires an application API key from
/// acoustid.org. Returns release candidates ready for [`fetch`].
pub fn acoustid_lookup(api_key: &str, fingerprint: &str, duration: u64) -> Result<Vec<Candidate>> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        anyhow::bail!("an AcoustID API key is required (Settings → General)");
    }
    if fingerprint.trim().is_empty() {
        anyhow::bail!("empty audio fingerprint");
    }
    let api = acoustid_api();
    let form = [
        ("client", api_key.to_string()),
        ("duration", duration.to_string()),
        ("fingerprint", fingerprint.to_string()),
        ("meta", "recordings+releases".to_string()),
        ("format", "json".to_string()),
    ];
    let parsed: AcoustIdResponse = post_form_json(&api, &form, "AcoustID lookup")?;
    if parsed.status.as_deref() == Some("error") {
        let message = parsed
            .error
            .map(|error| error.message)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "AcoustID returned an error".to_string());
        anyhow::bail!("{message}");
    }
    Ok(acoustid_candidates(parsed))
}

fn acoustid_candidates(parsed: AcoustIdResponse) -> Vec<Candidate> {
    let mut by_release: std::collections::HashMap<String, Candidate> =
        std::collections::HashMap::new();
    for result in parsed.results {
        let score = (result.score.clamp(0.0, 1.0) * 100.0).round() as u32;
        for recording in result.recordings {
            let artist = recording
                .artists
                .iter()
                .map(|artist| format!("{}{}", artist.name, artist.joinphrase))
                .collect::<String>()
                .trim()
                .to_string();
            for release in recording.releases {
                if release.id.is_empty() {
                    continue;
                }
                let candidate = Candidate {
                    release_id: release.id.clone(),
                    title: if release.title.is_empty() {
                        recording.title.clone()
                    } else {
                        release.title
                    },
                    artist: artist.clone(),
                    date: release
                        .date
                        .as_ref()
                        .and_then(AcoustIdDate::year)
                        .map(|year| year.to_string()),
                    country: release.country,
                    track_count: release.track_count.unwrap_or(0),
                    score,
                };
                match by_release.get(&release.id) {
                    Some(existing) if existing.score >= candidate.score => {}
                    _ => {
                        by_release.insert(release.id, candidate);
                    }
                }
            }
        }
    }

    let mut candidates: Vec<Candidate> = by_release.into_values().collect();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.title.cmp(&right.title))
    });
    candidates
}

static ACOUSTID_LAST: Mutex<Option<Instant>> = Mutex::new(None);

fn throttle_acoustid(url: &str) {
    if !url.starts_with("https://api.acoustid.org") {
        return;
    }
    let mut last = ACOUSTID_LAST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < Duration::from_millis(350) {
            std::thread::sleep(Duration::from_millis(350) - elapsed);
        }
    }
    *last = Some(Instant::now());
}

/// POST a form and decode JSON, retrying rate limits and server errors.
fn post_form_json<T: DeserializeOwned>(
    url: &str,
    form: &[(&str, String)],
    what: &str,
) -> Result<T> {
    const ATTEMPTS: u32 = 4;
    let mut last_error = None;

    for attempt in 1..=ATTEMPTS {
        throttle_acoustid(url);
        let response = match client()?.post(url).form(form).send() {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(anyhow::anyhow!("{what}: {error}"));
                std::thread::sleep(Duration::from_secs(attempt as u64));
                continue;
            }
        };
        let status = response.status();
        if status.is_success() {
            return response
                .json::<T>()
                .with_context(|| format!("cannot parse {what} reply"));
        }
        let retryable = status.as_u16() == 429 || status.is_server_error();
        let wait = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(attempt as u64 * 2);
        last_error = Some(anyhow::anyhow!("{what} returned {status}"));
        if !retryable || attempt == ATTEMPTS {
            break;
        }
        std::thread::sleep(Duration::from_secs(wait.min(10)));
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("{what} failed")))
}

// ---------------------------------------------------------------- release detail

#[derive(Deserialize)]
struct ReleaseResponse {
    #[serde(default)]
    title: String,
    date: Option<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
    #[serde(rename = "release-group")]
    release_group: Option<ReleaseGroup>,
    #[serde(default)]
    media: Vec<Media>,
    #[serde(rename = "cover-art-archive")]
    cover_art_archive: Option<CoverArtArchive>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    #[serde(default)]
    name: String,
    #[serde(default)]
    count: u32,
}

#[derive(Deserialize)]
struct Media {
    #[serde(default = "one")]
    position: u32,
    #[serde(default)]
    tracks: Vec<MediaTrack>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
struct MediaTrack {
    #[serde(default)]
    number: String,
    #[serde(default)]
    title: String,
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Deserialize)]
struct CoverArtArchive {
    #[serde(default)]
    front: bool,
}

fn year_of(date: &Option<String>) -> Option<i32> {
    date.as_deref()?.get(0..4)?.parse::<i32>().ok()
}

/// Fetch a release with its recordings.
pub fn fetch(release_id: &str) -> Result<Release> {
    let api = mb_api();
    let parsed: ReleaseResponse = get_json(
        &format!("{api}/release/{release_id}"),
        &[
            ("inc", "recordings+artist-credits+release-groups+tags"),
            ("fmt", "json"),
        ],
        "MusicBrainz release lookup",
    )?;
    Ok(release_from_response(release_id, parsed))
}

fn release_from_response(release_id: &str, parsed: ReleaseResponse) -> Release {
    let mut tracks = Vec::new();
    for media in &parsed.media {
        for (index, track) in media.tracks.iter().enumerate() {
            // MusicBrainz track numbers are strings like "3" or "A1".
            let number = track
                .number
                .parse::<u32>()
                .unwrap_or_else(|_| (index + 1) as u32);
            tracks.push(ReleaseTrack {
                disc: media.position.max(1),
                number,
                title: track.title.clone(),
                artist: credit_to_string(&track.artist_credit),
                length_ms: track.length,
            });
        }
    }

    let artist = credit_to_string(&parsed.artist_credit);
    let year = year_of(&parsed.date).or_else(|| {
        parsed
            .release_group
            .as_ref()
            .and_then(|g| year_of(&g.first_release_date))
    });
    let genre = parsed.release_group.as_ref().and_then(|group| {
        group
            .tags
            .iter()
            .max_by_key(|t| t.count)
            .filter(|t| !t.name.is_empty())
            .map(|t| t.name.clone())
    });

    Release {
        id: release_id.to_string(),
        title: parsed.title,
        artist: artist.clone(),
        album_artist: artist,
        year,
        genre,
        tracks,
        has_front_cover: parsed
            .cover_art_archive
            .map(|caa| caa.front)
            .unwrap_or(false),
    }
}

/// Download the front cover, if the archive has one.
pub fn front_cover(release_id: &str) -> Result<Option<Cover>> {
    // The archive normally serves `front-500`; fall back to the original image
    // for releases where that size is missing.
    let api = caa_api();
    for suffix in ["front-500", "front"] {
        if let Some(cover) = fetch_cover(&format!("{api}/release/{release_id}/{suffix}"))? {
            return Ok(Some(cover));
        }
    }
    Ok(None)
}

fn fetch_cover(url: &str) -> Result<Option<Cover>> {
    let mut response = None;
    for attempt in 0..3 {
        match client()?.get(url).send() {
            Ok(r) if r.status().is_success() => {
                response = Some(r);
                break;
            }
            Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                std::thread::sleep(Duration::from_secs(2 + attempt));
            }
            // 404 means the archive simply has no front cover for this release.
            _ => return Ok(None),
        }
    }
    let Some(response) = response else {
        return Ok(None);
    };

    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .to_string();

    let data =
        crate::bounded::read_capped(response, MAX_COVER_BYTES).context("cannot read cover art")?;
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(Cover { data, mime }))
}

// ---------------------------------------------------------------- matching

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn match_score(local: &LocalTrack, release: &ReleaseTrack) -> i32 {
    let mut score = 0;
    let local_title = normalize(&local.title);
    let release_title = normalize(&release.title);
    if !local_title.is_empty() && local_title == release_title {
        score += 70;
    } else if !local_title.is_empty()
        && !release_title.is_empty()
        && (local_title.contains(&release_title) || release_title.contains(&local_title))
    {
        score += 30;
    }

    let local_disc = local.disc_no.unwrap_or(1);
    if local.track_no == Some(release.number as i32) {
        score += if local_disc == release.disc as i32 {
            80
        } else {
            25
        };
    }

    if let Some(length_ms) = release.length_ms {
        let difference = (length_ms as i64 - local.duration_secs as i64 * 1000).abs();
        if difference <= 1_500 {
            score += 35;
        } else if difference <= 4_000 {
            score += 15;
        }
    }
    score
}

/// Suggest one release track per local file.
///
/// Unlike the old order-only pairing, exact disc/track numbers, titles and
/// durations are scored first. Files that still have no convincing match are
/// filled in disc/track/path order, which keeps untagged album folders usable.
/// `Some(index)` refers to `release.tracks`; `None` means no track was left.
pub fn suggest(local: &[LocalTrack], release: &Release) -> Vec<Option<usize>> {
    let mut result = vec![None; local.len()];
    let mut used = vec![false; release.tracks.len()];

    let mut pairs: Vec<(i32, usize, usize)> = Vec::new();
    for (local_index, item) in local.iter().enumerate() {
        for (release_index, track) in release.tracks.iter().enumerate() {
            let score = match_score(item, track);
            if score > 0 {
                pairs.push((score, local_index, release_index));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (score, local_index, release_index) in pairs {
        if score < 40 {
            break;
        }
        if result[local_index].is_some() || used[release_index] {
            continue;
        }
        result[local_index] = Some(release_index);
        used[release_index] = true;
    }

    // Fill the rest in the old, predictable order. This mostly matters for
    // files without track numbers or recognizable titles.
    let mut local_order: Vec<usize> = (0..local.len()).collect();
    local_order.sort_by(|&a, &b| {
        (
            local[a].disc_no.unwrap_or(1),
            local[a].track_no.unwrap_or(i32::MAX),
            &local[a].path,
        )
            .cmp(&(
                local[b].disc_no.unwrap_or(1),
                local[b].track_no.unwrap_or(i32::MAX),
                &local[b].path,
            ))
    });
    let mut release_order: Vec<usize> = (0..release.tracks.len()).collect();
    release_order.sort_by_key(|&index| (release.tracks[index].disc, release.tracks[index].number));

    let mut cursor = 0;
    for local_index in local_order {
        if result[local_index].is_some() {
            continue;
        }
        while cursor < release_order.len() && used[release_order[cursor]] {
            cursor += 1;
        }
        if let Some(&release_index) = release_order.get(cursor) {
            result[local_index] = Some(release_index);
            used[release_index] = true;
            cursor += 1;
        }
    }

    result
}

/// Pair local files with release tracks.
///
/// Order is the contract: local tracks are sorted by disc/track number and
/// release tracks by their own disc/track number, then zipped. Explicit
/// disc/track numbers are honoured when both sides have them, which keeps
/// multi-disc sets aligned even when file tags disagree with file names.
pub fn plan(local: &[LocalTrack], release: &Release) -> Vec<Match> {
    let mut local_order: Vec<&LocalTrack> = local.iter().collect();
    // Untagged files sort last by path, which keeps album folders in file-name
    // order instead of database order.
    local_order.sort_by(|a, b| {
        (
            a.disc_no.unwrap_or(1),
            a.track_no.unwrap_or(i32::MAX),
            &a.path,
        )
            .cmp(&(
                b.disc_no.unwrap_or(1),
                b.track_no.unwrap_or(i32::MAX),
                &b.path,
            ))
    });

    let mut release_order: Vec<&ReleaseTrack> = release.tracks.iter().collect();
    release_order.sort_by_key(|t| (t.disc, t.number));

    local_order
        .into_iter()
        .zip(release_order)
        .map(|(local, track)| Match {
            path: local.path.clone(),
            track: track.clone(),
        })
        .collect()
}

/// Rough quality signal for a candidate against what we have locally.
pub fn relevance(candidate: &Candidate, local_count: usize) -> i32 {
    let mut score = candidate.score as i32;
    if candidate.track_count == local_count && local_count > 0 {
        score += 25;
    } else if candidate.track_count > 0 {
        score -= (candidate.track_count as i32 - local_count as i32).abs() * 5;
    }
    score
}

// ---------------------------------------------------------------- writing

/// Write the matched release data into the files, optionally embedding art.
///
/// Returns the files that were updated. Errors are collected so one bad file
/// does not abort the whole album.
pub fn apply(release: &Release, matches: &[Match], cover: Option<&Cover>) -> (usize, Vec<String>) {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt;
    use lofty::prelude::{Accessor, ItemKey};
    use lofty::probe::Probe;
    use lofty::tag::{Tag, TagExt};

    let mut written = 0usize;
    let mut errors = Vec::new();

    let picture = cover.and_then(|cover| {
        let mime = cover_mime(&cover.mime)?;
        Some(lofty::picture::Picture::new_unchecked(
            lofty::picture::PictureType::CoverFront,
            Some(mime),
            None,
            cover.data.clone(),
        ))
    });

    for entry in matches {
        let result = (|| -> Result<()> {
            let mut tagged = Probe::open(&entry.path)
                .with_context(|| format!("cannot open {}", entry.path.display()))?
                .read()?;

            if tagged.primary_tag().is_none() {
                let tag_type = tagged.primary_tag_type();
                tagged.insert_tag(Tag::new(tag_type));
            }
            let tag = tagged
                .primary_tag_mut()
                .context("file has no writable tag")?;

            tag.set_title(entry.track.title.clone());
            tag.set_album(release.title.clone());
            tag.insert_text(ItemKey::AlbumArtist, release.album_artist.clone());
            if !entry.track.artist.is_empty() {
                tag.set_artist(entry.track.artist.clone());
            }
            tag.set_track(entry.track.number);
            tag.set_disk(entry.track.disc);
            if let Some(year) = release.year {
                tag.set_year(year as u32);
            }
            if let Some(genre) = &release.genre {
                tag.insert_text(ItemKey::Genre, genre.clone());
            }
            if let Some(picture) = &picture {
                tag.set_picture(0, picture.clone());
            }

            tag.save_to_path(&entry.path, WriteOptions::default())
                .with_context(|| format!("cannot write {}", entry.path.display()))?;
            Ok(())
        })();

        match result {
            Ok(()) => written += 1,
            Err(e) => errors.push(format!("{e:#}")),
        }
    }

    (written, errors)
}

fn cover_mime(mime: &str) -> Option<lofty::picture::MimeType> {
    match mime.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some(lofty::picture::MimeType::Jpeg),
        "image/png" => Some(lofty::picture::MimeType::Png),
        "image/gif" => Some(lofty::picture::MimeType::Gif),
        "image/bmp" => Some(lofty::picture::MimeType::Bmp),
        "image/tiff" => Some(lofty::picture::MimeType::Tiff),
        "image/webp" => Some(lofty::picture::MimeType::Unknown("image/webp".to_string())),
        _ => None,
    }
}

/// Save a cover next to the library as `cover.jpg`/`cover.png`, used when the
/// user prefers files over embedded art. Returns the path written.
pub fn save_cover_file(dir: &Path, cover: &Cover) -> Result<PathBuf> {
    let ext = match cover.mime.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/webp" => "webp",
        _ => "jpg",
    };
    let path = dir.join(format!("cover.{ext}"));
    std::fs::write(&path, &cover.data)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_JSON: &str = r#"{
      "count": 2,
      "releases": [
        {
          "id": "6a09041b-0f79-3278-88d9-0552dd28d37f",
          "score": 100,
          "title": "OK Computer",
          "date": "1997-07-01",
          "country": "US",
          "track-count": 12,
          "artist-credit": [
            {"name": "Radiohead", "joinphrase": ""}
          ]
        },
        {
          "id": "aaaaaaaa-1111-2222-3333-444444444444",
          "score": 62,
          "title": "OK Computer (remaster)",
          "artist-credit": [
            {"name": "Radiohead", "joinphrase": " feat. "},
            {"name": "Someone", "joinphrase": ""}
          ]
        }
      ]
    }"#;

    const RELEASE_JSON: &str = r#"{
      "title": "OK Computer",
      "date": "1997-07-01",
      "artist-credit": [{"name": "Radiohead", "joinphrase": ""}],
      "release-group": {
        "first-release-date": "1997-05-21",
        "tags": [{"name": "alternative rock", "count": 8}, {"name": "art rock", "count": 3}]
      },
      "media": [
        {
          "position": 1,
          "tracks": [
            {"number": "1", "title": "Airbag", "length": 284000,
             "artist-credit": [{"name": "Radiohead", "joinphrase": ""}]},
            {"number": "2", "title": "Paranoid Android", "length": 383000,
             "artist-credit": [{"name": "Radiohead", "joinphrase": ""}]}
          ]
        }
      ],
      "cover-art-archive": {"front": true}
    }"#;

    #[cfg(feature = "audio")]
    #[test]
    fn fingerprints_a_tone_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_tone_wav(&path, 3);
        let fingerprint = fingerprint(&path).unwrap();
        assert!(
            fingerprint.duration >= 2,
            "duration {}",
            fingerprint.duration
        );
        assert!(!fingerprint.fingerprint.is_empty());
        assert!(
            !fingerprint.fingerprint.contains(['+', '/', '=']),
            "fingerprint is not URL-safe base64: {}",
            fingerprint.fingerprint
        );
    }

    const ACOUSTID_JSON: &str = r#"{
      "status": "ok",
      "results": [
        {
          "score": 0.97,
          "recordings": [
            {
              "title": "Airbag",
              "artists": [{"name": "Radiohead", "joinphrase": ""}],
              "releases": [
                {"id": "release-a", "title": "OK Computer", "date": {"year": 1997}, "country": "GB", "track_count": 12},
                {"id": "release-b", "title": "OK Computer (Collector's Edition)", "date": "2009-06-01", "track_count": 24}
              ]
            }
          ]
        },
        {
          "score": 0.60,
          "recordings": [
            {
              "title": "Airbag",
              "artists": [{"name": "Radiohead", "joinphrase": ""}],
              "releases": [
                {"id": "release-a", "title": "OK Computer", "date": {"year": 1997}, "track_count": 12}
              ]
            }
          ]
        }
      ]
    }"#;

    #[test]
    fn parses_acoustid_candidates_and_deduplicates_releases() {
        let parsed: AcoustIdResponse = serde_json::from_str(ACOUSTID_JSON).unwrap();
        let candidates = acoustid_candidates(parsed);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].release_id, "release-a");
        assert_eq!(candidates[0].title, "OK Computer");
        assert_eq!(candidates[0].score, 97);
        assert_eq!(candidates[0].track_count, 12);
        assert_eq!(candidates[0].artist, "Radiohead");
        assert_eq!(candidates[0].date.as_deref(), Some("1997"));
        assert_eq!(candidates[1].release_id, "release-b");
        assert_eq!(candidates[1].title, "OK Computer (Collector's Edition)");
        assert_eq!(candidates[1].date.as_deref(), Some("2009"));
    }

    #[test]
    fn parses_search_results() {
        let parsed: SearchResponse = serde_json::from_str(SEARCH_JSON).unwrap();
        let candidates: Vec<Candidate> = parsed
            .releases
            .into_iter()
            .map(|r| Candidate {
                artist: credit_to_string(&r.artist_credit),
                release_id: r.id,
                title: r.title,
                date: r.date,
                country: r.country,
                track_count: r.track_count.unwrap_or(0),
                score: r.score,
            })
            .collect();

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].title, "OK Computer");
        assert_eq!(candidates[0].track_count, 12);
        assert_eq!(candidates[0].country.as_deref(), Some("US"));
        // Join phrases are preserved so "A feat. B" stays readable.
        assert_eq!(candidates[1].artist, "Radiohead feat. Someone");
        // A missing track-count must not fail parsing.
        assert_eq!(candidates[1].track_count, 0);
    }

    #[test]
    fn acoustid_lookup_posts_and_returns_candidates() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read the headers and enough of the form body to know the request
            // arrived; the response itself does not depend on it.
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let body = ACOUSTID_JSON;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            String::from_utf8_lossy(&request).to_string()
        });

        let previous = std::env::var_os("MUSICWASP_ACOUSTID_API");
        std::env::set_var(
            "MUSICWASP_ACOUSTID_API",
            format!("http://127.0.0.1:{port}/v2/lookup"),
        );
        let candidates = acoustid_lookup("test-key", "AQADfingerprint", 123).unwrap();
        match previous {
            Some(value) => std::env::set_var("MUSICWASP_ACOUSTID_API", value),
            None => std::env::remove_var("MUSICWASP_ACOUSTID_API"),
        }

        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v2/lookup"));
        assert!(request.contains("client=test-key"));
        assert!(request.contains("duration=123"));
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].release_id, "release-a");
        assert_eq!(candidates[0].score, 97);
    }

    #[test]
    fn parses_release_detail() {
        let parsed: ReleaseResponse = serde_json::from_str(RELEASE_JSON).unwrap();
        let release = release_from_response("mbid", parsed);

        assert_eq!(release.title, "OK Computer");
        assert_eq!(release.album_artist, "Radiohead");
        assert_eq!(release.year, Some(1997));
        assert_eq!(release.genre.as_deref(), Some("alternative rock"));
        assert!(release.has_front_cover);
        assert_eq!(release.tracks.len(), 2);
        assert_eq!(release.tracks[1].title, "Paranoid Android");
        assert_eq!(release.tracks[1].number, 2);
        assert_eq!(release.tracks[1].disc, 1);
        assert_eq!(release.tracks[1].length_ms, Some(383_000));
    }

    #[test]
    fn non_numeric_track_numbers_fall_back_to_position() {
        let json = r#"{
          "title": "Vinyl",
          "artist-credit": [{"name": "X"}],
          "media": [{"position": 2, "tracks": [
            {"number": "A1", "title": "Side A"},
            {"number": "A2", "title": "Side A two"}
          ]}]
        }"#;
        let parsed: ReleaseResponse = serde_json::from_str(json).unwrap();
        let release = release_from_response("id", parsed);
        assert_eq!(release.tracks[0].number, 1);
        assert_eq!(release.tracks[1].number, 2);
        assert_eq!(release.tracks[0].disc, 2);
    }

    fn local(title: &str, disc: i32, track: i32) -> LocalTrack {
        LocalTrack {
            path: PathBuf::from(format!("/music/{title}.flac")),
            disc_no: Some(disc),
            track_no: Some(track),
            title: title.to_string(),
            duration_secs: 200,
        }
    }

    fn release_with(tracks: Vec<ReleaseTrack>) -> Release {
        Release {
            id: "id".into(),
            title: "Album".into(),
            artist: "Artist".into(),
            album_artist: "Artist".into(),
            year: Some(2000),
            genre: None,
            tracks,
            has_front_cover: true,
        }
    }

    fn rt(disc: u32, number: u32, title: &str) -> ReleaseTrack {
        ReleaseTrack {
            disc,
            number,
            title: title.into(),
            artist: "Artist".into(),
            length_ms: None,
        }
    }

    #[test]
    fn plan_pairs_by_disc_and_position() {
        // Local files are deliberately out of order and the second disc comes
        // first, which is what a messy folder looks like.
        let local = vec![local("b2", 2, 2), local("a1", 1, 1), local("a2", 1, 2)];
        let release = release_with(vec![rt(1, 1, "One"), rt(1, 2, "Two"), rt(2, 1, "Three")]);

        let matches = plan(&local, &release);
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].track.title, "One");
        assert!(matches[0].path.ends_with("a1.flac"));
        assert_eq!(matches[1].track.title, "Two");
        assert!(matches[1].path.ends_with("a2.flac"));
        assert_eq!(matches[2].track.title, "Three");
        assert!(matches[2].path.ends_with("b2.flac"));
    }

    #[test]
    fn plan_orders_untagged_files_by_path() {
        let mut second = local("b", 1, 1);
        second.track_no = None;
        second.path = PathBuf::from("/music/02 second.mp3");
        let mut first = local("a", 1, 1);
        first.track_no = None;
        first.path = PathBuf::from("/music/01 first.mp3");

        let release = release_with(vec![rt(1, 1, "One"), rt(1, 2, "Two")]);
        let matches = plan(&[second, first], &release);

        assert_eq!(matches[0].track.title, "One");
        assert!(matches[0].path.ends_with("01 first.mp3"));
        assert!(matches[1].path.ends_with("02 second.mp3"));
    }

    #[test]
    fn plan_handles_fewer_local_tracks_than_the_release() {
        let local = vec![local("a1", 1, 1)];
        let release = release_with(vec![rt(1, 1, "One"), rt(1, 2, "Two")]);
        let matches = plan(&local, &release);
        assert_eq!(matches.len(), 1, "extra release tracks are ignored");
        assert_eq!(matches[0].track.title, "One");
    }

    #[test]
    fn suggest_matches_track_numbers_instead_of_input_order() {
        let local = vec![local("two", 1, 2), local("one", 1, 1)];
        let release = release_with(vec![rt(1, 1, "One"), rt(1, 2, "Two")]);
        assert_eq!(suggest(&local, &release), vec![Some(1), Some(0)]);
    }

    #[test]
    fn suggest_falls_back_to_path_order_for_untagged_files() {
        let mut second = local("zzz", 1, 1);
        second.track_no = None;
        second.path = PathBuf::from("/music/02 second.mp3");
        let mut first = local("yyy", 1, 1);
        first.track_no = None;
        first.path = PathBuf::from("/music/01 first.mp3");

        let release = release_with(vec![rt(1, 1, "One"), rt(1, 2, "Two")]);
        let mapping = suggest(&[second, first], &release);
        assert_eq!(mapping, vec![Some(1), Some(0)]);
    }

    #[test]
    fn suggest_leaves_extra_local_files_unmapped() {
        let local = vec![local("one", 1, 1), local("extra", 1, 2)];
        let release = release_with(vec![rt(1, 1, "One")]);
        assert_eq!(suggest(&local, &release), vec![Some(0), None]);
    }

    #[test]
    fn relevance_prefers_matching_track_counts() {
        let exact = Candidate {
            release_id: "a".into(),
            title: "Album".into(),
            artist: "Artist".into(),
            date: None,
            country: None,
            track_count: 10,
            score: 80,
        };
        let wrong = Candidate {
            track_count: 16,
            score: 90,
            ..exact.clone()
        };
        assert!(relevance(&exact, 10) > relevance(&wrong, 10));
    }

    #[test]
    fn writing_tags_and_cover_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.wav");
        write_wav(&path);

        let release = Release {
            id: "id".into(),
            title: "The Album".into(),
            artist: "The Artist".into(),
            album_artist: "The Artist".into(),
            year: Some(1999),
            genre: Some("Ambient".into()),
            tracks: vec![rt(1, 1, "The Song")],
            has_front_cover: true,
        };
        let matches = vec![Match {
            path: path.clone(),
            track: release.tracks[0].clone(),
        }];

        let cover = Cover {
            data: vec![
                0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00,
                0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
            ],
            mime: "image/jpeg".into(),
        };

        let (written, errors) = apply(&release, &matches, Some(&cover));
        assert_eq!(written, 1, "errors: {errors:?}");

        // Re-read through the library's tag reader.
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;
        let probe = lofty::probe::Probe::open(&path).unwrap().read().unwrap();
        let tag = probe.primary_tag().or_else(|| probe.first_tag()).unwrap();
        assert_eq!(tag.title().as_deref(), Some("The Song"));
        assert_eq!(tag.album().as_deref(), Some("The Album"));
        assert_eq!(tag.year(), Some(1999));
        assert_eq!(tag.track(), Some(1));
        assert_eq!(tag.pictures().len(), 1, "cover is embedded");

        // And as a side-car file.
        let cover_path = save_cover_file(dir.path(), &cover).unwrap();
        assert!(cover_path.ends_with("cover.jpg"));
        assert_eq!(
            std::fs::metadata(cover_path).unwrap().len(),
            cover.data.len() as u64
        );
    }

    #[cfg(feature = "audio")]
    fn write_tone_wav(path: &Path, seconds: u32) {
        use std::io::Write;
        let sample_rate = 44_100u32;
        let frames = sample_rate * seconds;
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + frames * 2).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&(sample_rate * 2).to_le_bytes()).unwrap();
        f.write_all(&2u16.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&(frames * 2).to_le_bytes()).unwrap();
        for frame in 0..frames {
            let value = ((frame as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32).sin()
                * 8000.0) as i16;
            f.write_all(&value.to_le_bytes()).unwrap();
        }
    }

    fn write_wav(path: &Path) {
        use std::io::Write;
        let frames = 800u32;
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + frames).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&44_100u32.to_le_bytes()).unwrap();
        f.write_all(&44_100u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&8u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&frames.to_le_bytes()).unwrap();
        f.write_all(&vec![0u8; frames as usize]).unwrap();
    }
}
