//! Debug-only screenshot and initial-view hooks.

use super::*;
use crate::debug_hooks as debug_env;

pub(super) fn apply_initial(app: &mut App) {
    if let Some(view) = debug_env::var("MUSICWASP_VIEW") {
        app.view = match view.as_str() {
            "podcasts" => View::Podcasts,
            "radio" => View::Radio,
            _ => app.view,
        };
    }
    if app.view == View::Podcasts && debug_env::is_set("MUSICWASP_PODCAST_FIRST") {
        if let Some(feed) = app
            .podcast_feeds
            .iter()
            .find(|feed| feed.episode_count > 0)
            .or_else(|| app.podcast_feeds.first())
        {
            app.select_podcast_feed(feed.id);
        }
    }
    if debug_env::is_set("MUSICWASP_LYRICS") {
        app.settings.lyrics_open = true;
    }
}

impl App {
    pub(super) fn handle_screenshot_capture(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot_path.clone() else {
            return;
        };
        self.frames += 1;

        // Debug hook: MUSICWASP_AUTOPLAY=1 starts the first track (muted).
        if self.frames == 15 && debug_env::is_set("MUSICWASP_AUTOPLAY") && !self.tracks.is_empty() {
            self.volume = 0.0;
            self.play_from_list(0);
        }

        if self.frames == 20 {
            if let Some(seconds) = debug_env::var("MUSICWASP_SEEK") {
                if let (Ok(seconds), Some(player)) = (seconds.parse::<f64>(), &self.player) {
                    player.seek(seconds);
                }
            }
        }

        if self.frames == 15
            && debug_env::is_set("MUSICWASP_PLAY_RADIO")
            && !self.radio_stations.is_empty()
        {
            let station = self.radio_stations[0].clone();
            self.play_radio_station(&station);
        }
        if self.frames == 15 && debug_env::is_set("MUSICWASP_PLAY_PODCAST") {
            if let Some(feed) = self
                .podcast_feeds
                .iter()
                .find(|feed| feed.episode_count > 0)
            {
                let feed_id = feed.id;
                self.select_podcast_feed(feed_id);
                if let Some(episode) = self.podcast_episodes.first().cloned() {
                    self.play_podcast_episode(&episode);
                }
            }
        }

        // Debug hook: MUSICWASP_SETTINGS=1 opens the settings panel;
        // MUSICWASP_SETTINGS_TAB=keybindings|about picks a tab.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_SETTINGS") {
            self.show_settings = true;
            self.settings_tab = match debug_env::var("MUSICWASP_SETTINGS_TAB").as_deref() {
                Some("keybindings") => SettingsTab::Keybindings,
                Some("about") => SettingsTab::About,
                Some("look") => SettingsTab::LookAndFeel,
                Some("tracks") => SettingsTab::Tracks,
                Some("visualizer") => SettingsTab::Visualizer,
                _ => SettingsTab::General,
            };
        }

        // Debug hook: MUSICWASP_EDITOR=1 opens the tag editor for the first
        // track; MUSICWASP_EDITOR=multi selects everything first.
        if self.frames == 20 {
            match debug_env::var("MUSICWASP_EDITOR").as_deref() {
                Some("multi") => {
                    self.select_all_in_view();
                    self.open_editor();
                }
                Some(_) if !self.tracks.is_empty() => {
                    self.selected.clear();
                    self.selected.insert(self.tracks[0].id);
                    self.anchor = Some(self.tracks[0].id);
                    self.open_editor();
                }
                _ => {}
            }
        }

        // Debug hook: MUSICWASP_RENAME=1 opens rename for the first track title.
        if self.frames == 20
            && debug_env::is_set("MUSICWASP_RENAME")
            && self.rename.is_none()
            && !self.tracks.is_empty()
        {
            let id = self.tracks[0].id;
            self.selected.clear();
            self.selected.insert(id);
            self.anchor = Some(id);
            self.rename_field = TrackField::Title;
            self.focus = Focus::Tracks;
            self.start_rename();
        }

        // Debug hook: MUSICWASP_AUTOTAG=1 opens the dialog and searches;
        // MUSICWASP_AUTOTAG=one tags just the first track.
        if self.frames == 20 && !self.tracks.is_empty() {
            match debug_env::var("MUSICWASP_AUTOTAG").as_deref() {
                Some("one") => {
                    let id = self.tracks[0].id;
                    self.selected.clear();
                    self.selected.insert(id);
                    self.anchor = Some(id);
                    self.open_autotag();
                    self.start_autotag_search();
                }
                Some(_) => {
                    self.select_all_in_view();
                    self.open_autotag();
                    self.start_autotag_search();
                }
                None => {}
            }
        }

        // Debug hook: MUSICWASP_AUTOTAG_APPLY=1 applies the loaded candidate
        // (paired with MUSICWASP_AUTOTAG=one).
        if debug_env::is_set("MUSICWASP_AUTOTAG_APPLY") && self.frames > 20 {
            let ready = self.autotag.as_ref().is_some_and(|state| {
                matches!(state.stage, AutoTagStage::Results(_))
                    && state.release.is_some()
                    && !state.loading_release
            });
            if ready {
                self.start_autotag_apply();
            }
        }

        // Debug hook: MUSICWASP_SMART=1 creates; =edit opens the first existing.
        if self.frames == 20 {
            match debug_env::var("MUSICWASP_SMART").as_deref() {
                Some("edit") => {
                    if let Some(smart) = self.smart_playlists.first() {
                        self.edit_smart_dialog(smart.id);
                    }
                }
                Some(_) => self.open_smart_dialog(),
                None => {}
            }
        }

        // Debug hook: MUSICWASP_PLAYLIST=1 (or =smart) opens a playlist view.
        if self.frames == 20 {
            match debug_env::var("MUSICWASP_PLAYLIST").as_deref() {
                Some("smart") => {
                    if let Some(first) = self.smart_playlists.first() {
                        self.view = View::Smart(first.id);
                        self.mark_tracks_dirty();
                    }
                }
                Some(_) => {
                    if let Some(first) = self.playlists.first() {
                        self.view = View::Playlist(first.id);
                        self.mark_tracks_dirty();
                    }
                }
                None => {}
            }
        }

        // Debug hook: MUSICWASP_PICKER=1 opens the folder chooser.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_PICKER") {
            self.open_picker();
        }

        // Debug hook: MUSICWASP_EQ=1 opens the equalizer.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_EQ") {
            self.show_equalizer = true;
        }

        // Debug hook: MUSICWASP_COLUMNS=1 opens the column manager.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_COLUMNS") {
            self.show_columns = true;
        }

        // Debug hook: MUSICWASP_SCAN=/path starts a background scan.
        if self.frames == 20 {
            if let Some(path) = debug_env::var("MUSICWASP_SCAN") {
                self.scan_path = path;
                self.scan_clicked();
            }
        }

        // Debug hook: MUSICWASP_UNDO=1 restores the last tag snapshot.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_UNDO") {
            self.undo_last_tag_write();
        }

        // Debug hook: MUSICWASP_ALBUM_GRID=1 opens the cover grid.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_ALBUM_GRID") {
            self.album_view = true;
            self.albums_dirty = true;
        }

        // Debug hook: MUSICWASP_HYGIENE=1 opens the library hygiene window.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_HYGIENE") {
            self.open_hygiene();
        }

        if self.frames == 30 && debug_env::is_set("MUSICWASP_MINI") {
            self.set_mini_player(true, ctx);
        }

        // Debug hook: MUSICWASP_RESTORE_DIALOG=1 makes a backup and opens it.
        if self.frames == 30 && debug_env::is_set("MUSICWASP_RESTORE_DIALOG") {
            let _ = self.backup_library();
            self.open_restore_dialog();
        }

        // Debug hook: MUSICWASP_SKIN_EDITOR=1 opens the colour editor.
        if self.frames == 30 && debug_env::is_set("MUSICWASP_SKIN_EDITOR") {
            self.open_skin_editor();
        }

        // Debug hook: MUSICWASP_ORGANIZE=1 opens the file organizer preview.
        if self.frames == 20 && debug_env::is_set("MUSICWASP_ORGANIZE") {
            self.open_organize();
        }

        // Debug hook: MUSICWASP_RECONCILE=1 also runs the rename preview.
        if self.frames == 30 && debug_env::is_set("MUSICWASP_RECONCILE") {
            self.show_hygiene = true;
            self.reconcile_hygiene();
        }

        let shot_frame = debug_env::var("MUSICWASP_SCREENSHOT_FRAME")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(40);

        if self.frames == shot_frame && !self.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.shot_requested = true;
            return;
        }

        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });

        if let Some(img) = image {
            let [w, h] = img.size;
            if let Some(buf) = image::RgbaImage::from_raw(
                w as u32,
                h as u32,
                img.pixels.iter().flat_map(|c| c.to_array()).collect(),
            ) {
                let _ = buf.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        ctx.request_repaint();
    }
}
