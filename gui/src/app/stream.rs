//! Radio/podcast stream playback and radio station management.

use super::*;
use musicwasp_core::podcast::PodcastEpisode;
use musicwasp_core::radio::RadioStation;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Radio,
    Podcast,
}

/// One internet radio station or podcast episode currently playing.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub title: String,
    pub subtitle: String,
    pub url: String,
    pub kind: StreamKind,
    pub episode_id: Option<i64>,
    pub duration_secs: u64,
}

#[derive(Default)]
pub struct RadioState {
    pub stations: Vec<RadioStation>,
    pub status: Option<String>,
    pub show_dialog: bool,
    pub name: String,
    pub url: String,
    pub genre: String,
}

pub struct StreamState {
    pub info: Option<Arc<StreamInfo>>,
    pub started: std::time::Instant,
    pub saved: std::time::Instant,
}

impl Default for StreamState {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self {
            info: None,
            started: now,
            saved: now,
        }
    }
}

impl App {
    pub fn refresh_radio_stations(&mut self) {
        match self.lib.radio_stations() {
            Ok(stations) => self.radio.stations = stations,
            Err(error) => self.radio.status = Some(format!("could not load stations: {error}")),
        }
    }

    pub fn add_radio_station(&mut self, name: &str, url: &str, genre: &str) {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            self.radio.status = Some("enter an http(s) stream URL".to_string());
            return;
        }
        if name.trim().is_empty() {
            self.radio.status = Some("enter a station name".to_string());
            return;
        }
        match self.lib.add_radio_station(name, url, genre) {
            Ok(_) => {
                self.radio.show_dialog = false;
                self.radio.name.clear();
                self.radio.url.clear();
                self.radio.genre.clear();
                self.radio.status = Some("station saved".to_string());
                self.refresh_radio_stations();
            }
            Err(error) => self.radio.status = Some(format!("could not save station: {error}")),
        }
    }

    pub fn delete_radio_station(&mut self, id: i64) {
        match self.lib.delete_radio_station(id) {
            Ok(()) => self.refresh_radio_stations(),
            Err(error) => self.radio.status = Some(format!("could not delete station: {error}")),
        }
    }

    pub fn play_radio_station(&mut self, station: &RadioStation) {
        self.play_stream(StreamInfo {
            title: station.name.clone(),
            subtitle: if station.genre.trim().is_empty() {
                "Internet radio".to_string()
            } else {
                station.genre.clone()
            },
            url: station.url.clone(),
            kind: StreamKind::Radio,
            episode_id: None,
            duration_secs: 0,
        });
    }

    pub fn play_podcast_episode(&mut self, episode: &PodcastEpisode) {
        let feed_title = self
            .podcast
            .feeds
            .iter()
            .find(|feed| feed.id == episode.feed_id)
            .map(|feed| feed.title.clone())
            .unwrap_or_else(|| "Podcast".to_string());
        self.play_stream(StreamInfo {
            title: episode.title.clone(),
            subtitle: feed_title,
            url: episode.audio_url.clone(),
            kind: StreamKind::Podcast,
            episode_id: Some(episode.id),
            duration_secs: episode.duration_secs,
        });
    }

    fn play_stream(&mut self, stream: StreamInfo) {
        self.finalize_stream();
        self.queue.clear();
        self.queue_pos = None;
        self.playing_id = None;
        self.art = None;
        self.art_key = None;
        self.gapless_for = None;
        self.selected.clear();
        self.finish_scrobble();
        if let Some(player) = &self.player {
            // Radio streams are URLs the user typed. Podcast episodes come from
            // the feed, so they follow the local-feeds setting.
            let access = match stream.kind {
                StreamKind::Radio => musicwasp_core::netguard::Access::AnyHost,
                StreamKind::Podcast => self.podcast_access(),
            };
            player.load_url(&stream.url, access);
        }
        self.message = match stream.kind {
            StreamKind::Radio => format!("Radio: {}", stream.title),
            StreamKind::Podcast => format!("Podcast: {}", stream.title),
        };
        self.stream.info = Some(Arc::new(stream));
        self.stream.started = std::time::Instant::now();
        self.stream.saved = std::time::Instant::now();
    }

    /// Save podcast listening position without clearing the stream.
    pub(super) fn save_stream_progress(&mut self, finished: bool) {
        let Some(stream) = &self.stream.info else {
            return;
        };
        let Some(episode_id) = stream.episode_id else {
            return;
        };
        let position = self
            .player
            .as_ref()
            .map(|player| player.snapshot().position.max(0.0) as u64)
            .unwrap_or(0);
        let played = finished
            || (stream.duration_secs > 0 && position.saturating_add(15) >= stream.duration_secs);
        if let Err(error) = self.lib.set_podcast_progress(episode_id, position, played) {
            eprintln!("MusicWasp: could not save podcast progress: {error}");
        }
        if let Some(episode) = self
            .podcast
            .episodes
            .iter_mut()
            .find(|episode| episode.id == episode_id)
        {
            episode.position_secs = position;
            episode.played |= played;
        }
    }

    pub(super) fn finalize_stream(&mut self) {
        if self.stream.info.is_some() {
            self.save_stream_progress(false);
            self.stream.info = None;
        }
    }

    /// Persist podcast position every few seconds and when playback stops.
    pub fn update_stream_progress(&mut self) {
        let Some(stream) = &self.stream.info else {
            return;
        };
        if stream.episode_id.is_none() {
            return;
        }
        let Some(player) = &self.player else {
            return;
        };
        let snapshot = player.snapshot();
        let loaded = snapshot
            .current
            .as_ref()
            .is_some_and(|path| path.to_string_lossy() == stream.url);
        if !loaded && self.stream.started.elapsed() < std::time::Duration::from_secs(10) {
            // The audio thread may still be opening the HTTP stream.
            return;
        }
        if snapshot.status == musicwasp_core::PlayerStatus::Stopped {
            // The natural end of an episode goes through `on_track_finished`;
            // stopping here means a failed or dropped stream.
            self.save_stream_progress(false);
            self.stream.info = None;
            self.message = "stream stopped".to_string();
            return;
        }
        if self.stream.saved.elapsed() >= std::time::Duration::from_secs(5) {
            self.save_stream_progress(false);
            self.stream.saved = std::time::Instant::now();
        }
    }
}
