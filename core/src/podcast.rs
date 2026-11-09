//! Podcast subscriptions: RSS fetching, parsing and persistence.
//!
//! Feed refreshes happen on background threads in the GUI; this module only
//! performs one blocking fetch/parse and the SQLite updates.

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use rusqlite::{params, Connection};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const USER_AGENT: &str = concat!(
    "MusicWasp/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/xavier/music-wasp )"
);

#[derive(Debug, Clone, Default)]
pub struct ParsedFeed {
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub episodes: Vec<ParsedEpisode>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedEpisode {
    pub guid: String,
    pub title: String,
    pub description: String,
    pub audio_url: String,
    pub image_url: String,
    pub published: String,
    /// Days since the Unix epoch, used only for ordering.
    pub published_at: i64,
    pub duration_secs: u64,
}

#[derive(Debug, Clone)]
pub struct PodcastFeed {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub last_checked: Option<i64>,
    pub episode_count: usize,
    pub unplayed_count: usize,
}

#[derive(Debug, Clone)]
pub struct PodcastEpisode {
    pub id: i64,
    pub feed_id: i64,
    pub guid: String,
    pub title: String,
    pub description: String,
    pub audio_url: String,
    pub image_url: String,
    pub published: String,
    pub published_at: i64,
    pub duration_secs: u64,
    pub played: bool,
    pub position_secs: u64,
}

/// Largest feed accepted. Real feeds are a few MB; this stops a hostile server
/// from exhausting memory.
const MAX_FEED_BYTES: u64 = 32 * 1024 * 1024;

/// Download and parse one feed. The caller runs this off the UI thread.
pub fn fetch(url: &str) -> Result<ParsedFeed> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()
        .context("cannot build HTTP client")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("cannot fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("{url} returned an error"))?;
    let bytes = crate::bounded::read_capped(response, MAX_FEED_BYTES)
        .with_context(|| format!("cannot read {url}"))?;
    let mut feed = parse(&bytes)?;
    if feed.title.trim().is_empty() {
        feed.title = url.to_string();
    }
    Ok(feed)
}

pub fn parse(bytes: &[u8]) -> Result<ParsedFeed> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Target {
        FeedTitle,
        FeedDescription,
        FeedImage,
        ItemTitle,
        ItemDescription,
        ItemGuid,
        ItemPublished,
        ItemDuration,
    }

    let text = std::str::from_utf8(bytes).context("feed is not UTF-8")?;
    let mut reader = Reader::from_str(text);
    let mut feed = ParsedFeed::default();
    let mut item = ParsedEpisode::default();
    let mut in_item = false;
    let mut target: Option<Target> = None;
    let mut buffer = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
                let name = event.name();
                let name = name.as_ref();
                match name {
                    b"item" | b"entry" => {
                        in_item = true;
                        item = ParsedEpisode::default();
                    }
                    b"title" => {
                        target = Some(if in_item {
                            Target::ItemTitle
                        } else {
                            Target::FeedTitle
                        });
                        buffer.clear();
                    }
                    b"description" | b"itunes:summary" | b"content:encoded" | b"summary"
                    | b"content" => {
                        target = Some(if in_item {
                            Target::ItemDescription
                        } else {
                            Target::FeedDescription
                        });
                        buffer.clear();
                    }
                    b"subtitle" => {
                        target = Some(Target::FeedDescription);
                        buffer.clear();
                    }
                    b"guid" | b"id" if in_item => {
                        target = Some(Target::ItemGuid);
                        buffer.clear();
                    }
                    b"pubDate" | b"published" | b"updated" if in_item => {
                        target = Some(Target::ItemPublished);
                        buffer.clear();
                    }
                    b"itunes:duration" if in_item => {
                        target = Some(Target::ItemDuration);
                        buffer.clear();
                    }
                    b"url" | b"icon" | b"logo" => {
                        // Inside the channel-level <image> or Atom feed icons.
                        if !in_item {
                            target = Some(Target::FeedImage);
                            buffer.clear();
                        }
                    }
                    b"link" if in_item => {
                        let rel = attribute(&event, b"rel").unwrap_or_default();
                        if rel == "enclosure" {
                            if let Some(href) = attribute(&event, b"href") {
                                let mime = attribute(&event, b"type").unwrap_or_default();
                                if mime.is_empty() || mime.starts_with("audio/") {
                                    item.audio_url = href;
                                }
                            }
                        }
                    }
                    b"itunes:image" => {
                        if let Some(href) = attribute(&event, b"href") {
                            if in_item {
                                item.image_url = href;
                            } else {
                                feed.image_url = href;
                            }
                        }
                    }
                    b"enclosure" if in_item => {
                        if let Some(url) = attribute(&event, b"url") {
                            let mime = attribute(&event, b"type").unwrap_or_default();
                            if mime.is_empty() || mime.starts_with("audio/") {
                                item.audio_url = url;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(event)) => {
                if target.is_some() {
                    if let Ok(text) = event.decode() {
                        buffer.push_str(text.as_ref());
                    }
                }
            }
            Ok(Event::CData(event)) => {
                if target.is_some() {
                    if let Ok(text) = event.decode() {
                        buffer.push_str(text.as_ref());
                    }
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if target.is_some() {
                    match reference.resolve_char_ref() {
                        Ok(Some(character)) => buffer.push(character),
                        _ => {
                            if let Ok(name) = reference.decode() {
                                buffer.push_str(match name.as_ref() {
                                    "amp" => "&",
                                    "lt" => "<",
                                    "gt" => ">",
                                    "quot" => "\"",
                                    "apos" => "'",
                                    _ => "",
                                });
                            }
                        }
                    }
                }
            }
            Ok(Event::End(event)) => {
                let name = event.name();
                let name = name.as_ref();
                let current = match name {
                    b"title" => Some(if in_item {
                        Target::ItemTitle
                    } else {
                        Target::FeedTitle
                    }),
                    b"description" | b"itunes:summary" | b"content:encoded" | b"summary"
                    | b"content" => Some(if in_item {
                        Target::ItemDescription
                    } else {
                        Target::FeedDescription
                    }),
                    b"subtitle" => Some(Target::FeedDescription),
                    b"guid" | b"id" if in_item => Some(Target::ItemGuid),
                    b"pubDate" | b"published" | b"updated" if in_item => {
                        Some(Target::ItemPublished)
                    }
                    b"itunes:duration" if in_item => Some(Target::ItemDuration),
                    b"url" | b"icon" | b"logo" if !in_item => Some(Target::FeedImage),
                    _ => None,
                };
                if let Some(current) = current {
                    if target == Some(current) {
                        let value = buffer.trim().to_string();
                        match current {
                            Target::FeedTitle => feed.title = value,
                            Target::FeedDescription => feed.description = plain_text(&value),
                            Target::FeedImage => feed.image_url = value,
                            Target::ItemTitle => item.title = value,
                            Target::ItemDescription => item.description = plain_text(&value),
                            Target::ItemGuid => item.guid = value,
                            Target::ItemPublished => item.published = value,
                            Target::ItemDuration => item.duration_secs = duration_secs(&value),
                        }
                        target = None;
                        buffer.clear();
                    }
                }
                if name == b"item" || name == b"entry" {
                    if !item.audio_url.trim().is_empty() {
                        if item.guid.trim().is_empty() {
                            item.guid = item.audio_url.clone();
                        }
                        item.published_at = date_key(&item.published);
                        feed.episodes.push(item.clone());
                    }
                    in_item = false;
                    item = ParsedEpisode::default();
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                anyhow::bail!("invalid feed XML at {}: {error}", reader.buffer_position())
            }
        }
    }

    Ok(feed)
}

fn attribute(event: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == name {
            return attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|value| value.to_string());
        }
    }
    None
}

fn plain_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn duration_secs(value: &str) -> u64 {
    let mut parts = value
        .trim()
        .split(':')
        .filter_map(|part| part.parse::<u64>().ok());
    match value.matches(':').count() {
        2 => {
            let (Some(hours), Some(minutes), Some(seconds)) =
                (parts.next(), parts.next(), parts.next())
            else {
                return 0;
            };
            hours * 3600 + minutes * 60 + seconds
        }
        1 => {
            let (Some(minutes), Some(seconds)) = (parts.next(), parts.next()) else {
                return 0;
            };
            minutes * 60 + seconds
        }
        _ => value.trim().parse().unwrap_or(0),
    }
}

fn date_key(value: &str) -> i64 {
    let value = value.trim();
    // ISO-8601 dates, common in Atom feeds.
    if value.len() >= 10 {
        let head = &value[..10];
        if let (Ok(year), Ok(month), Ok(day)) = (
            head[0..4].parse::<i64>(),
            head[5..7].parse::<i64>(),
            head[8..10].parse::<i64>(),
        ) {
            return days_from_civil(year, month, day);
        }
    }
    // RFC 2822: "Tue, 01 Jan 2024 12:00:00 +0000".
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let month = tokens.iter().position(|token| {
        matches!(
            token.to_ascii_lowercase().as_str(),
            "jan"
                | "feb"
                | "mar"
                | "apr"
                | "may"
                | "jun"
                | "jul"
                | "aug"
                | "sep"
                | "oct"
                | "nov"
                | "dec"
        )
    });
    let Some(index) = month else {
        return 0;
    };
    let month = index as i64 + 1;
    let day = tokens
        .get(index.wrapping_sub(1))
        .and_then(|token| token.trim_end_matches(',').parse::<i64>().ok())
        .unwrap_or(0);
    let year = tokens
        .get(index + 1)
        .and_then(|token| token.parse::<i64>().ok())
        .unwrap_or(0);
    if year == 0 || day == 0 {
        0
    } else {
        days_from_civil(year, month, day)
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

pub fn feeds(conn: &Connection) -> Result<Vec<PodcastFeed>> {
    let mut statement = conn.prepare(
        "SELECT f.id, f.url, f.title, f.description, f.image_url, f.last_checked,
                COUNT(e.id),
                COALESCE(SUM(CASE WHEN e.played = 0 THEN 1 ELSE 0 END), 0)
         FROM podcast_feeds f
         LEFT JOIN podcast_episodes e ON e.feed_id = f.id
         GROUP BY f.id
         ORDER BY LOWER(f.title), f.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PodcastFeed {
            id: row.get(0)?,
            url: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            image_url: row.get(4)?,
            last_checked: row.get(5)?,
            episode_count: row.get::<_, i64>(6)? as usize,
            unplayed_count: row.get::<_, i64>(7)? as usize,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn add_feed(conn: &Connection, url: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO podcast_feeds (url, title) VALUES (?1, ?1)
         ON CONFLICT(url) DO UPDATE SET url = excluded.url",
        [url],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM podcast_feeds WHERE url = ?1",
        [url],
        |row| row.get(0),
    )?)
}

/// Store a refresh, preserving per-episode played state and progress.
pub fn update_feed(conn: &Connection, feed_id: i64, feed: &ParsedFeed) -> Result<()> {
    let title = if feed.title.trim().is_empty() {
        "Untitled feed"
    } else {
        feed.title.trim()
    };
    conn.execute(
        "UPDATE podcast_feeds
         SET title = ?1, description = ?2, image_url = ?3, last_checked = ?4
         WHERE id = ?5",
        params![
            title,
            plain_text(&feed.description),
            feed.image_url,
            unix_now(),
            feed_id
        ],
    )?;
    for episode in &feed.episodes {
        conn.execute(
            "INSERT INTO podcast_episodes
                (feed_id, guid, title, description, audio_url, image_url,
                 published, published_at, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(feed_id, guid) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                audio_url = excluded.audio_url,
                image_url = excluded.image_url,
                published = excluded.published,
                published_at = excluded.published_at,
                duration_secs = excluded.duration_secs",
            params![
                feed_id,
                episode.guid,
                episode.title,
                plain_text(&episode.description),
                episode.audio_url,
                episode.image_url,
                episode.published,
                episode.published_at,
                episode.duration_secs as i64,
            ],
        )?;
    }
    Ok(())
}

pub fn episodes(conn: &Connection, feed_id: i64) -> Result<Vec<PodcastEpisode>> {
    let mut statement = conn.prepare(
        "SELECT id, feed_id, guid, title, description, audio_url, image_url,
                published, published_at, duration_secs, played, position_secs
         FROM podcast_episodes
         WHERE feed_id = ?1
         ORDER BY published_at DESC, id DESC",
    )?;
    let rows = statement.query_map([feed_id], |row| {
        Ok(PodcastEpisode {
            id: row.get(0)?,
            feed_id: row.get(1)?,
            guid: row.get(2)?,
            title: row.get(3)?,
            description: row.get(4)?,
            audio_url: row.get(5)?,
            image_url: row.get(6)?,
            published: row.get(7)?,
            published_at: row.get(8)?,
            duration_secs: row.get::<_, i64>(9)?.max(0) as u64,
            played: row.get::<_, i64>(10)? != 0,
            position_secs: row.get::<_, i64>(11)?.max(0) as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_feed(conn: &Connection, feed_id: i64) -> Result<()> {
    conn.execute("DELETE FROM podcast_feeds WHERE id = ?1", [feed_id])?;
    Ok(())
}

pub fn set_progress(conn: &Connection, episode_id: i64, position: u64, played: bool) -> Result<()> {
    conn.execute(
        "UPDATE podcast_episodes SET position_secs = ?1, played = ?2 WHERE id = ?3",
        params![position as i64, i64::from(played), episode_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
      <channel>
        <title>Example Show</title>
        <description><![CDATA[A <b>great</b> show]]></description>
        <image><url>https://example.com/feed.png</url></image>
        <itunes:image href="https://example.com/feed2.png"/>
        <item>
          <title>Episode One</title>
          <guid isPermaLink="false">ep-1</guid>
          <description>First &amp; best</description>
          <pubDate>Tue, 02 Jan 2024 10:00:00 +0000</pubDate>
          <itunes:duration>1:02:03</itunes:duration>
          <enclosure url="https://example.com/one.mp3" type="audio/mpeg" length="123"/>
          <itunes:image href="https://example.com/one.png"/>
        </item>
        <item>
          <title>Episode Two</title>
          <guid>ep-2</guid>
          <pubDate>Wed, 03 Jan 2024 10:00:00 +0000</pubDate>
          <itunes:duration>245</itunes:duration>
          <enclosure url="https://example.com/two.m4a" type="audio/mp4"/>
        </item>
      </channel>
    </rss>"#;

    #[test]
    fn parses_rss_podcast() {
        let feed = parse(FEED.as_bytes()).unwrap();
        assert_eq!(feed.title, "Example Show");
        assert_eq!(feed.description, "A great show");
        assert_eq!(feed.image_url, "https://example.com/feed2.png");
        assert_eq!(feed.episodes.len(), 2);
        assert_eq!(feed.episodes[0].title, "Episode One");
        assert_eq!(feed.episodes[0].audio_url, "https://example.com/one.mp3");
        assert_eq!(feed.episodes[0].duration_secs, 3723);
        assert_eq!(feed.episodes[0].description, "First & best");
        assert_eq!(feed.episodes[0].image_url, "https://example.com/one.png");
        assert_eq!(feed.episodes[1].duration_secs, 245);
        assert!(feed.episodes[0].published_at > 0);
        assert!(feed.episodes[1].published_at > feed.episodes[0].published_at);
    }

    #[test]
    fn parses_atom_feed() {
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom">
          <title>Atom Show</title>
          <subtitle>Atom description</subtitle>
          <icon>https://example.com/atom.png</icon>
          <entry>
            <title>Atom Episode</title>
            <id>atom-1</id>
            <published>2024-01-03T10:00:00Z</published>
            <summary>Atom summary</summary>
            <link rel="enclosure" href="https://example.com/atom.mp3" type="audio/mpeg"/>
          </entry>
        </feed>"#;
        let feed = parse(atom.as_bytes()).unwrap();
        assert_eq!(feed.title, "Atom Show");
        assert_eq!(feed.description, "Atom description");
        assert_eq!(feed.image_url, "https://example.com/atom.png");
        assert_eq!(feed.episodes.len(), 1);
        assert_eq!(feed.episodes[0].title, "Atom Episode");
        assert_eq!(feed.episodes[0].guid, "atom-1");
        assert_eq!(feed.episodes[0].audio_url, "https://example.com/atom.mp3");
        assert_eq!(feed.episodes[0].description, "Atom summary");
        assert!(feed.episodes[0].published_at > 0);
    }

    #[test]
    fn ignores_items_without_audio() {
        let feed = parse(
            br#"<rss><channel><title>X</title><item><title>No audio</title></item></channel></rss>"#,
        )
        .unwrap();
        assert!(feed.episodes.is_empty());
    }
}
