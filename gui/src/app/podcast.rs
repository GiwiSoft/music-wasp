//! Podcast feed fetching, selection and storage.

use super::*;

pub enum PodcastMsg {
    Feed {
        feed_id: i64,
        result: Result<ParsedFeed, String>,
    },
}

impl App {
    pub fn refresh_podcast_feeds(&mut self) {
        match self.lib.podcast_feeds() {
            Ok(feeds) => {
                self.podcast_feeds = feeds;
                if let Some(id) = self.selected_feed {
                    if self.podcast_feeds.iter().any(|feed| feed.id == id) {
                        self.load_podcast_episodes();
                    } else {
                        self.selected_feed = None;
                        self.podcast_episodes.clear();
                    }
                }
            }
            Err(error) => self.podcast_status = Some(format!("could not load feeds: {error}")),
        }
    }

    pub fn load_podcast_episodes(&mut self) {
        let Some(feed_id) = self.selected_feed else {
            self.podcast_episodes.clear();
            return;
        };
        match self.lib.podcast_episodes(feed_id) {
            Ok(episodes) => self.podcast_episodes = episodes,
            Err(error) => self.podcast_status = Some(format!("could not load episodes: {error}")),
        }
    }

    pub fn select_podcast_feed(&mut self, feed_id: i64) {
        self.selected_feed = Some(feed_id);
        self.load_podcast_episodes();
    }

    pub fn add_podcast_feed(&mut self, url: &str) {
        let url = url.trim().to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            self.podcast_status = Some("enter an http(s) feed URL".to_string());
            return;
        }
        if self.podcast_loading {
            self.podcast_status = Some("a feed refresh is already running".to_string());
            return;
        }
        match self.lib.add_podcast_feed(&url) {
            Ok(feed_id) => {
                self.selected_feed = Some(feed_id);
                self.show_podcast_dialog = false;
                self.podcast_url.clear();
                self.start_podcast_fetch(feed_id, url);
            }
            Err(error) => self.podcast_status = Some(format!("could not save feed: {error}")),
        }
    }

    pub fn refresh_selected_podcast(&mut self) {
        let Some(feed_id) = self.selected_feed else {
            return;
        };
        if self.podcast_loading {
            return;
        }
        let Some(feed) = self.podcast_feeds.iter().find(|feed| feed.id == feed_id) else {
            return;
        };
        let url = feed.url.clone();
        self.start_podcast_fetch(feed_id, url);
    }

    /// Network reach for podcast feeds and episodes, from the user's setting.
    pub fn podcast_access(&self) -> musicwasp_core::netguard::Access {
        if self.settings.allow_local_feeds {
            musicwasp_core::netguard::Access::AnyHost
        } else {
            musicwasp_core::netguard::Access::PublicOnly
        }
    }

    fn start_podcast_fetch(&mut self, feed_id: i64, url: String) {
        let (tx, rx) = mpsc::channel();
        self.podcast_rx = Some(rx);
        self.podcast_loading = true;
        self.podcast_status = Some("fetching feed…".to_string());
        let access = self.podcast_access();
        std::thread::spawn(move || {
            let result =
                musicwasp_core::podcast::fetch(&url, access).map_err(|error| format!("{error:#}"));
            let _ = tx.send(PodcastMsg::Feed { feed_id, result });
        });
    }

    pub fn poll_podcasts(&mut self) {
        let Some(rx) = self.podcast_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(PodcastMsg::Feed { feed_id, result }) => {
                self.podcast_loading = false;
                match result {
                    Ok(feed) => match self.lib.update_podcast_feed(feed_id, &feed) {
                        Ok(()) => {
                            self.podcast_status =
                                Some(format!("{} episode(s)", feed.episodes.len()));
                            self.refresh_podcast_feeds();
                            if self.selected_feed == Some(feed_id) {
                                self.load_podcast_episodes();
                            }
                        }
                        Err(error) => {
                            self.podcast_status = Some(format!("could not save feed: {error}"))
                        }
                    },
                    Err(error) => self.podcast_status = Some(format!("feed failed: {error}")),
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.podcast_rx = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.podcast_loading = false;
                self.podcast_status = Some("feed worker stopped".to_string());
            }
        }
    }

    pub fn delete_podcast_feed(&mut self, feed_id: i64) {
        if let Err(error) = self.lib.delete_podcast_feed(feed_id) {
            self.podcast_status = Some(format!("could not delete feed: {error}"));
            return;
        }
        if self.selected_feed == Some(feed_id) {
            self.selected_feed = None;
            self.podcast_episodes.clear();
        }
        self.refresh_podcast_feeds();
    }
}
