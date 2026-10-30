use crate::app::{fmt_size, fmt_time, App, Focus, RenameTarget, SettingsTab, TrackField};
use crate::icons;
use crate::theme as th;
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};
use musicwasp_core::{SortKey, Track};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

/// True while files are being dragged from the OS and the pointer is over `rect`.
fn os_files_over(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.ctx().input(|i| {
        !i.raw.hovered_files.is_empty() && i.pointer.hover_pos().is_some_and(|p| rect.contains(p))
    })
}

/// Shorten `text` so it fits `width` pixels, ending with a hyphen when the text
/// had to be cut. Used by the queue so long titles/artists crop instead of
/// spilling out of the panel.
fn crop_to_width(ui: &egui::Ui, text: &str, width: f32, size: f32) -> String {
    let font = egui::FontId::proportional(size);
    let measure = |value: &str| {
        ui.painter()
            .layout_no_wrap(value.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    crop_with_hyphen(text, width, measure("-"), &measure)
}

/// Width-measuring core of [`crop_to_width`], kept separate so it can be tested
/// without an egui context.
fn crop_with_hyphen(
    text: &str,
    width: f32,
    hyphen_width: f32,
    measure: &dyn Fn(&str) -> f32,
) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    // Binary search the longest prefix whose width still leaves room for the
    // hyphen. Measures are memoised but still allocate, so avoid one per char.
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect();
        if measure(&candidate) + hyphen_width <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut cropped: String = chars[..lo].iter().collect();
    let trimmed = cropped.trim_end().len();
    cropped.truncate(trimmed);
    cropped.push('-');
    cropped
}

/// Small caret that folds a browser section. Returns true when clicked.
fn fold_button(ui: &mut egui::Ui, folded: bool, tooltip: &str) -> bool {
    let icon = if folded {
        icons::CARET_RIGHT
    } else {
        icons::CARET_DOWN
    };
    ui.add(
        egui::Button::new(icons::text(icon, 12.0))
            .frame(false)
            .min_size(egui::vec2(18.0, 18.0)),
    )
    .on_hover_text(tooltip)
    .clicked()
}

/// Section title; clicking it also folds the section.
fn section_label(ui: &mut egui::Ui, icon: egui_phosphor_icons::Icon, title: &str) -> bool {
    let text = RichText::new(format!("{} {}", icon.as_str(), title))
        .size(10.0)
        .strong()
        .color(th::text_dim());
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
        .clicked()
}

pub fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            if let Some(logo) = &app.logo {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                ui.painter().image(
                    logo.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ui.label(
                RichText::new("MusicWasp")
                    .size(19.0)
                    .strong()
                    .color(th::accent()),
            );
            ui.label(RichText::new("v0.1").size(10.0).color(th::text_dim()));
            ui.separator();
            let mut view_label = app.view_title();
            if view_label.chars().count() > 26 {
                view_label = view_label.chars().take(25).collect::<String>() + "…";
            }
            ui.label(RichText::new(view_label).size(12.0).color(th::text()));
            ui.separator();

            ui.label(icons::text(icons::MAGNIFYING_GLASS, 13.0).color(th::text_dim()));
            let search = ui.add_sized(
                [180.0, 24.0],
                egui::TextEdit::singleline(&mut app.search)
                    .id(egui::Id::new("search_box"))
                    .hint_text("search title, artist, album…"),
            );
            if search.changed() {
                app.mark_tracks_dirty();
            }
            if app.focus_search {
                search.request_focus();
                app.focus_search = false;
            }

            ui.separator();
            ui.add_sized(
                [210.0, 24.0],
                egui::TextEdit::singleline(&mut app.scan_path)
                    .id(egui::Id::new("scan_path"))
                    .hint_text("folder to scan"),
            );
            if icons::compact(ui, icons::FOLDER_OPEN, "Choose a music folder").clicked() {
                app.open_picker();
            }
            if icons::button(ui, icons::ARROWS_CLOCKWISE, "Scan")
                .on_hover_text("Add every track in the folder to the library")
                .clicked()
            {
                app.scan_clicked();
            }

            let selected = app.selected_in_view().len();

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::compact(
                    ui,
                    icons::GEAR,
                    "Settings: skins, keyboard shortcuts and preferences",
                )
                .clicked()
                {
                    app.show_settings = !app.show_settings;
                }
                if icons::compact(
                    ui,
                    icons::BROOM,
                    "Library hygiene: missing files, duplicates, prune",
                )
                .clicked()
                {
                    app.open_hygiene();
                }
                if icons::toggle(
                    ui,
                    app.plain_view,
                    icons::MONITOR_PLAY,
                    if app.plain_view {
                        "Back to the library"
                    } else {
                        "Plain now-playing view"
                    },
                )
                .clicked()
                {
                    app.plain_view = !app.plain_view;
                }
                if !app.plain_view
                    && icons::toggle(
                        ui,
                        app.settings.queue_open,
                        icons::QUEUE,
                        "Play queue: show or hide Up Next",
                    )
                    .clicked()
                {
                    app.settings.queue_open = !app.settings.queue_open;
                    app.persist_settings();
                }
                if icons::toggle(
                    ui,
                    app.show_equalizer,
                    icons::SLIDERS,
                    "Equalizer / DSP (Ctrl+E)",
                )
                .clicked()
                {
                    app.show_equalizer = !app.show_equalizer;
                }
                ui.separator();
                if !app.message.is_empty() {
                    if ui.available_width() > 180.0 {
                        ui.label(RichText::new(&app.message).size(11.0).color(th::accent()));
                    }
                } else if ui.available_width() < 210.0 {
                    // Not enough room: the table already shows what matters.
                } else if selected > 1 {
                    ui.label(
                        RichText::new(format!("{selected} selected"))
                            .size(11.0)
                            .color(th::accent()),
                    );
                } else if let Ok(s) = app.lib.stats() {
                    ui.label(
                        RichText::new(format!(
                            "{} tracks · {} albums · {} artists · {}",
                            s.tracks,
                            s.albums,
                            s.artists,
                            fmt_size(s.total_bytes)
                        ))
                        .size(11.0)
                        .color(th::text_dim()),
                    );
                }
            });
        });
        ui.add_space(5.0);
    });
}

pub fn browser(app: &mut App, ui: &mut egui::Ui) {
    enum Act {
        Genre(Option<String>),
        Artist(Option<String>),
        Album(Option<String>),
        Library,
        Playlist(i64),
        Smart(i64),
        DropOnPlaylist(i64, Vec<i64>, Option<i64>),
        SaveView,
        NewSmart,
        DeletePlaylist,
    }
    let mut act: Option<Act> = None;
    let mut os_zones: Vec<crate::app::OsDropZone> = Vec::new();
    let mut folded = app.settings.folded;
    let mut folded_changed = false;

    egui::Panel::left("browser")
        .resizable(true)
        .default_size(210.0)
        .min_size(150.0)
        .show(ui, |ui| {
            ui.add_space(4.0);
            let full_h = ui.available_height();

            let sections = [
                (icons::FUNNEL_SIMPLE, "GENRE", 0.25),
                (icons::PERSON, "ARTIST", 0.31),
                (icons::VINYL_RECORD, "ALBUM", 0.24),
            ];

            // Expanded sections share the available height proportionally, so
            // folding one gives its space to the others.
            let mut total_weight = if folded.playlists { 0.0 } else { 0.20 };
            if !folded.genre {
                total_weight += 0.25;
            }
            if !folded.artist {
                total_weight += 0.31;
            }
            if !folded.album {
                total_weight += 0.24;
            }
            let header_h = 22.0;
            let usable = (full_h - header_h * 4.0 - 12.0).max(0.0);
            let height_for = |weight: f32| {
                if total_weight > 0.0 {
                    (usable * weight / total_weight).max(60.0)
                } else {
                    60.0
                }
            };

            for (i, (icon, title, weight)) in sections.iter().enumerate() {
                let start_folded = match i {
                    0 => folded.genre,
                    1 => folded.artist,
                    _ => folded.album,
                };
                let mut toggled = false;
                ui.horizontal(|ui| {
                    toggled |= fold_button(
                        ui,
                        start_folded,
                        if start_folded {
                            "Expand this section"
                        } else {
                            "Collapse this section"
                        },
                    );
                    toggled |= section_label(ui, *icon, title);
                });
                if toggled {
                    match i {
                        0 => folded.genre = !folded.genre,
                        1 => folded.artist = !folded.artist,
                        _ => folded.album = !folded.album,
                    }
                    folded_changed = true;
                }

                let is_folded = match i {
                    0 => folded.genre,
                    1 => folded.artist,
                    _ => folded.album,
                };
                if !is_folded {
                    let (items, selected): (&[String], Option<&String>) = match i {
                        0 => (&app.genres, app.sel_genre.as_ref()),
                        1 => (&app.artists, app.sel_artist.as_ref()),
                        _ => (&app.albums, app.sel_album.as_ref()),
                    };
                    egui::ScrollArea::vertical()
                        .id_salt(*title)
                        .auto_shrink([false, true])
                        .max_height(height_for(*weight))
                        .show(ui, |ui| {
                            if ui.selectable_label(selected.is_none(), "All").clicked() {
                                act = Some(match i {
                                    0 => Act::Genre(None),
                                    1 => Act::Artist(None),
                                    _ => Act::Album(None),
                                });
                            }
                            for item in items {
                                let sel = selected == Some(item);
                                if ui.selectable_label(sel, item).clicked() {
                                    let value = if sel { None } else { Some(item.clone()) };
                                    act = Some(match i {
                                        0 => Act::Genre(value),
                                        1 => Act::Artist(value),
                                        _ => Act::Album(value),
                                    });
                                }
                            }
                        });
                }
                ui.separator();
            }

            // Playlists
            let mut playlists_toggled = false;
            ui.horizontal(|ui| {
                playlists_toggled |= fold_button(
                    ui,
                    folded.playlists,
                    if folded.playlists {
                        "Expand playlists"
                    } else {
                        "Collapse playlists"
                    },
                );
                playlists_toggled |= section_label(ui, icons::QUEUE, "PLAYLISTS");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::compact(ui, icons::PLUS, "Save the current view as a playlist")
                        .clicked()
                    {
                        act = Some(Act::SaveView);
                    }
                    if icons::compact(ui, icons::SPARKLE, "New smart playlist").clicked() {
                        act = Some(Act::NewSmart);
                    }
                    if matches!(
                        app.view,
                        crate::app::View::Playlist(_) | crate::app::View::Smart(_)
                    ) && icons::compact(ui, icons::TRASH, "Delete playlist").clicked()
                    {
                        act = Some(Act::DeletePlaylist);
                    }
                });
            });
            if playlists_toggled {
                folded.playlists = !folded.playlists;
                folded_changed = true;
            }

            if !folded.playlists {
                egui::ScrollArea::vertical()
                    .id_salt("PLAYLISTS")
                    .auto_shrink([false, true])
                    .max_height(height_for(0.20).max(70.0))
                    .show(ui, |ui| {
                        let in_library = matches!(app.view, crate::app::View::Library);
                        let all = ui.selectable_label(
                            in_library,
                            icons::with(icons::MUSIC_NOTES, "All music"),
                        );
                        os_zones.push(crate::app::OsDropZone {
                            playlist: None,
                            rect: all.rect,
                        });
                        if os_files_over(ui, all.rect) {
                            ui.painter().rect_stroke(
                                all.rect,
                                3.0,
                                egui::Stroke::new(1.5_f32, th::accent()),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if all.clicked() {
                            act = Some(Act::Library);
                        }
                        for playlist in &app.playlists {
                            let selected = matches!(app.view, crate::app::View::Playlist(id) if id == playlist.id);
                            let label = format!(
                                "{} {} ({})",
                                icons::PLAYLIST.as_str(),
                                playlist.name,
                                playlist.count
                            );
                            let resp = ui.selectable_label(selected, label);
                            os_zones.push(crate::app::OsDropZone {
                                playlist: Some(playlist.id),
                                rect: resp.rect,
                            });

                            // Drop target for dragged tracks and OS files.
                            if resp
                                .dnd_hover_payload::<crate::app::DragTracks>()
                                .is_some()
                                || os_files_over(ui, resp.rect)
                            {
                                ui.painter().rect_stroke(
                                    resp.rect,
                                    3.0,
                                    egui::Stroke::new(1.5_f32, th::accent()),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if let Some(drag) =
                                resp.dnd_release_payload::<crate::app::DragTracks>()
                            {
                                act = Some(Act::DropOnPlaylist(
                                    playlist.id,
                                    drag.ids.clone(),
                                    drag.from_playlist,
                                ));
                            }

                            if resp.clicked() {
                                act = Some(Act::Playlist(playlist.id));
                            }
                        }
                        for smart in &app.smart_playlists {
                            let selected = matches!(app.view, crate::app::View::Smart(id) if id == smart.id);
                            let label = format!("{} {}", icons::SPARKLE.as_str(), smart.name);
                            if ui.selectable_label(selected, label).clicked() {
                                act = Some(Act::Smart(smart.id));
                            }
                        }

                        if app.playlists.is_empty() && app.smart_playlists.is_empty() {
                            ui.label(
                                RichText::new("no playlists yet")
                                    .size(10.0)
                                    .color(th::text_dim()),
                            );
                        } else if egui::DragAndDrop::has_any_payload(ui.ctx()) {
                            ui.label(
                                RichText::new("drop tracks here")
                                    .size(9.0)
                                    .color(th::accent()),
                            );
                        }
                    });
            }
        });

    if folded_changed {
        app.settings.folded = folded;
        app.persist_settings();
    }

    app.os_drop_zones = os_zones;

    if act.is_some() {
        app.focus = Focus::Browser;
    }

    match act {
        Some(Act::Genre(v)) => {
            app.sel_genre = v;
            app.view = crate::app::View::Library;
            app.mark_browser_dirty();
        }
        Some(Act::Artist(v)) => {
            app.sel_artist = v;
            app.view = crate::app::View::Library;
            app.mark_browser_dirty();
        }
        Some(Act::Album(v)) => {
            app.sel_album = v;
            app.view = crate::app::View::Library;
            app.mark_tracks_dirty();
        }
        Some(Act::Library) => {
            app.view = crate::app::View::Library;
            app.mark_tracks_dirty();
        }
        Some(Act::Playlist(id)) => {
            app.view = crate::app::View::Playlist(id);
            app.mark_tracks_dirty();
        }
        Some(Act::Smart(id)) => {
            app.view = crate::app::View::Smart(id);
            app.mark_tracks_dirty();
        }
        Some(Act::DropOnPlaylist(playlist_id, ids, from)) => {
            app.transfer_to_playlist(playlist_id, &ids, from);
        }
        Some(Act::SaveView) => {
            let n = app.playlists.len() + 1;
            app.playlist_dialog = Some(format!("Playlist {n}"));
        }
        Some(Act::NewSmart) => app.open_smart_dialog(),
        Some(Act::DeletePlaylist) => app.delete_current_playlist(),
        None => {}
    }
}

pub fn transport(app: &mut App, ui: &mut egui::Ui) {
    let snap =
        app.player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or(musicwasp_core::PlayerSnapshot {
                status: musicwasp_core::PlayerStatus::Stopped,
                position: 0.0,
                duration: 0.0,
                volume: app.volume,
                current: None,
                finished: false,
            });

    let current_track = snap
        .current
        .as_ref()
        .and_then(|p| app.lib.track_by_path(p).ok().flatten());

    // Trust the tag reader's duration over the decoder's estimate.
    let known_duration = current_track
        .as_ref()
        .map(|t| t.duration_secs as f64)
        .filter(|d| *d > 0.0);
    let duration = known_duration.unwrap_or(snap.duration);

    egui::Panel::bottom("transport").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            // Album art
            let (rect, _) = ui.allocate_exact_size(egui::vec2(62.0, 62.0), egui::Sense::hover());
            match &app.art {
                Some((_, tex)) => {
                    ui.painter().image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, 3.0, th::bg_row_alt());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icons::MUSIC_NOTES.as_str(),
                        egui::FontId::proportional(28.0),
                        th::text_dim(),
                    );
                }
            }

            ui.add_space(8.0);
            ui.vertical(|ui| {
                let title = current_track
                    .as_ref()
                    .map(|t| t.title.clone())
                    .unwrap_or_else(|| "Nothing playing".to_string());
                let sub = current_track
                    .as_ref()
                    .map(|t| format!("{}  ·  {}", t.artist, t.album))
                    .unwrap_or_else(|| "Double-click a track to play".to_string());
                ui.label(RichText::new(title).size(15.0).strong().color(th::text()));
                ui.label(RichText::new(sub).size(11.0).color(th::text_dim()));

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if icons::transport(ui, icons::SKIP_BACK, 16.0, false, "Previous").clicked() {
                        app.prev_track();
                    }
                    let playing = snap.status == musicwasp_core::PlayerStatus::Playing;
                    let icon = if playing { icons::PAUSE } else { icons::PLAY };
                    if icons::transport(ui, icon, 18.0, true, "Play / pause").clicked() {
                        app.toggle_pause();
                    }
                    if icons::transport(ui, icons::SKIP_FORWARD, 16.0, false, "Next").clicked() {
                        app.next_track();
                    }
                    if icons::transport(ui, icons::STOP, 14.0, false, "Stop").clicked() {
                        app.stop();
                    }
                    ui.separator();

                    let repeat_icon = if app.repeat == crate::app::RepeatMode::One {
                        icons::REPEAT_ONCE
                    } else {
                        icons::REPEAT
                    };
                    if icons::toggle(
                        ui,
                        app.repeat != crate::app::RepeatMode::Off,
                        repeat_icon,
                        app.repeat.label(),
                    )
                    .clicked()
                    {
                        app.repeat = app.repeat.next();
                    }
                    if icons::toggle(ui, app.shuffle, icons::SHUFFLE, "Shuffle").clicked() {
                        app.shuffle = !app.shuffle;
                    }
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                ui.spacing_mut().slider_width = 120.0;
                ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                ui.label(icons::text(icons::SPEAKER_HIGH, 14.0).color(th::text_dim()))
                    .on_hover_text("Volume");
            });
        });

        ui.add_space(6.0);

        // Seek row: elapsed | slider | total
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(fmt_time(app.seek_drag.unwrap_or(snap.position)))
                    .monospace()
                    .size(11.0)
                    .color(th::text_dim()),
            );
            let mut pos = app.seek_drag.unwrap_or(snap.position);
            let max = duration.max(1.0);
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(100.0);
            let resp = ui.add(
                egui::Slider::new(&mut pos, 0.0..=max)
                    .show_value(false)
                    .trailing_fill(true),
            );
            if resp.dragged() {
                app.seek_drag = Some(pos);
            }
            if resp.drag_stopped() {
                if let Some(p) = &app.player {
                    p.seek(pos);
                }
                app.seek_drag = None;
            }
            ui.label(
                RichText::new(fmt_time(duration))
                    .monospace()
                    .size(11.0)
                    .color(th::text_dim()),
            );
        });

        if let Some(err) = &app.audio_error {
            ui.label(
                RichText::new(format!("audio unavailable: {err}"))
                    .size(10.0)
                    .color(Color32::from_rgb(220, 110, 110)),
            );
        }
        ui.add_space(4.0);
    });

    // Volume changed by the slider above is pushed to the engine next frame.
    if let Some(p) = &app.player {
        if (app.volume - app.volume_sent).abs() > f32::EPSILON {
            p.set_volume(app.volume);
            app.volume_sent = app.volume;
        }
    }
}

/// Full-window now-playing view: big cover, transport and the next track.
pub fn now_playing(app: &mut App, ui: &mut egui::Ui) {
    let snap =
        app.player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or(musicwasp_core::PlayerSnapshot {
                status: musicwasp_core::PlayerStatus::Stopped,
                position: 0.0,
                duration: 0.0,
                volume: app.volume,
                current: None,
                finished: false,
            });
    let current_track = snap
        .current
        .as_ref()
        .and_then(|p| app.lib.track_by_path(p).ok().flatten());
    let known_duration = current_track
        .as_ref()
        .map(|t| t.duration_secs as f64)
        .filter(|d| *d > 0.0);
    let duration = known_duration.unwrap_or(snap.duration);
    let upcoming = app.upcoming_track();
    let playing = snap.status == musicwasp_core::PlayerStatus::Playing;

    egui::CentralPanel::default().show(ui, |ui| {
        let avail = ui.available_size();
        let art = (avail.y * 0.40).clamp(140.0, 330.0).min(avail.x * 0.42);

        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(art, art), egui::Sense::hover());
            match &app.art {
                Some((_, tex)) => {
                    ui.painter().image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, 6.0, th::bg_row_alt());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icons::MUSIC_NOTES.as_str(),
                        egui::FontId::proportional((art * 0.30).clamp(32.0, 96.0)),
                        th::text_dim(),
                    );
                }
            }
            ui.painter().rect_stroke(
                rect,
                6.0,
                egui::Stroke::new(1.0_f32, th::bg_selected()),
                egui::StrokeKind::Inside,
            );

            ui.add_space(18.0);
            let title = current_track
                .as_ref()
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "Nothing playing".to_string());
            ui.label(RichText::new(title).size(26.0).strong().color(th::text()));

            ui.add_space(2.0);
            let subtitle = current_track
                .as_ref()
                .map(|t| {
                    if t.album.is_empty() {
                        t.artist.clone()
                    } else {
                        format!("{}  ·  {}", t.artist, t.album)
                    }
                })
                .unwrap_or_else(|| "Pick a track in the library to start".to_string());
            ui.label(RichText::new(subtitle).size(14.0).color(th::text_dim()));

            ui.add_space(16.0);
            let seek_row = (avail.x * 0.46).clamp(360.0, 620.0);
            ui.allocate_ui(egui::vec2(seek_row, 24.0), |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(fmt_time(app.seek_drag.unwrap_or(snap.position)))
                            .monospace()
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                    let mut pos = app.seek_drag.unwrap_or(snap.position);
                    let max = duration.max(1.0);
                    ui.spacing_mut().slider_width = (seek_row - 100.0).max(160.0);
                    let resp = ui.add(
                        egui::Slider::new(&mut pos, 0.0..=max)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if resp.dragged() {
                        app.seek_drag = Some(pos);
                    }
                    if resp.drag_stopped() {
                        if let Some(p) = &app.player {
                            p.seek(pos);
                        }
                        app.seek_drag = None;
                    }
                    ui.label(
                        RichText::new(fmt_time(duration))
                            .monospace()
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                });
            });

            ui.add_space(12.0);
            ui.allocate_ui(egui::vec2(285.0, 34.0), |ui| {
                ui.horizontal(|ui| {
                    if icons::transport(ui, icons::SKIP_BACK, 22.0, false, "Previous").clicked() {
                        app.prev_track();
                    }
                    let icon = if playing { icons::PAUSE } else { icons::PLAY };
                    if icons::transport(ui, icon, 30.0, true, "Play / pause").clicked() {
                        app.toggle_pause();
                    }
                    if icons::transport(ui, icons::SKIP_FORWARD, 22.0, false, "Next").clicked() {
                        app.next_track();
                    }
                    if icons::transport(ui, icons::STOP, 18.0, false, "Stop").clicked() {
                        app.stop();
                    }
                    let repeat_icon = if app.repeat == crate::app::RepeatMode::One {
                        icons::REPEAT_ONCE
                    } else {
                        icons::REPEAT
                    };
                    if icons::toggle(
                        ui,
                        app.repeat != crate::app::RepeatMode::Off,
                        repeat_icon,
                        app.repeat.label(),
                    )
                    .clicked()
                    {
                        app.repeat = app.repeat.next();
                    }
                    if icons::toggle(ui, app.shuffle, icons::SHUFFLE, "Shuffle").clicked() {
                        app.shuffle = !app.shuffle;
                    }
                });
            });

            ui.add_space(12.0);
            ui.allocate_ui(egui::vec2(160.0, 24.0), |ui| {
                ui.horizontal(|ui| {
                    ui.label(icons::text(icons::SPEAKER_HIGH, 14.0).color(th::text_dim()));
                    ui.spacing_mut().slider_width = 120.0;
                    ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                });
            });

            ui.add_space(26.0);
            match &upcoming {
                Some(track) => {
                    ui.label(
                        RichText::new(format!("{} UP NEXT", icons::QUEUE.as_str()))
                            .size(10.0)
                            .strong()
                            .color(th::text_dim()),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(&track.title).size(15.0).color(th::text()));
                    ui.label(
                        RichText::new(format!("{}  ·  {}", track.artist, track.album))
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                }
                None => {
                    ui.label(
                        RichText::new(format!("{} end of queue", icons::CHECK.as_str()))
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                }
            }

            if let Some(err) = &app.audio_error {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "{} audio unavailable: {err}",
                        icons::WARNING.as_str()
                    ))
                    .size(10.0)
                    .color(Color32::from_rgb(220, 110, 110)),
                );
            }
        });
    });

    if let Some(p) = &app.player {
        if (app.volume - app.volume_sent).abs() > f32::EPSILON {
            p.set_volume(app.volume);
            app.volume_sent = app.volume;
        }
    }
}

/// Right-hand play queue: the current track and the editable Up Next list.
pub fn queue_panel(app: &mut App, ui: &mut egui::Ui) {
    if !app.settings.queue_open {
        return;
    }
    let ctx = ui.ctx().clone();

    enum Act {
        Jump(usize),
        Remove(usize),
        Move { from: usize, to: usize },
        Clear,
        Close,
    }
    let mut act: Option<Act> = None;

    // Snapshot the queue so the UI can borrow tracks without holding a mutable
    // borrow of the app.
    let queue = app.queue.clone();
    let pos = app.queue_pos;
    let current_id = pos.and_then(|pos| queue.get(pos).copied());
    let upcoming: Vec<(usize, i64)> = queue
        .iter()
        .copied()
        .enumerate()
        .skip(pos.map(|pos| pos + 1).unwrap_or(0))
        .collect();

    let can_clear = !upcoming.is_empty();

    // Borrow the visible tracks only; missing ones are fetched in one small
    // batch for the rows currently on screen.
    {
        let local: HashMap<i64, &Track> =
            app.tracks.iter().map(|track| (track.id, track)).collect();
        let current_track = current_id
            .and_then(|id| local.get(&id).map(|track| (*track).clone()))
            .or_else(|| {
                current_id.and_then(|id| {
                    app.lib
                        .tracks_by_ids(&[id])
                        .ok()
                        .and_then(|mut tracks| tracks.pop())
                })
            });

        egui::Panel::right("queue")
            .resizable(true)
            .default_size(286.0)
            .min_size(220.0)
            .max_size(440.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} QUEUE", icons::QUEUE.as_str()))
                            .size(10.0)
                            .strong()
                            .color(th::text_dim()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::compact(ui, icons::X, "Hide the play queue").clicked() {
                            act = Some(Act::Close);
                        }
                        if icons::compact_enabled(
                            ui,
                            can_clear,
                            icons::TRASH,
                            "Clear everything after the current track",
                        )
                        .clicked()
                        {
                            act = Some(Act::Clear);
                        }
                    });
                });
                ui.separator();

                if let Some(track) = &current_track {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("NOW PLAYING")
                            .size(9.5)
                            .strong()
                            .color(th::accent()),
                    );
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&track.title)
                                .size(13.0)
                                .strong()
                                .color(th::text()),
                        )
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  ·  {}", track.artist, track.album))
                                .size(10.5)
                                .color(th::text_dim()),
                        )
                        .truncate(),
                    );
                    ui.add_space(6.0);
                    ui.separator();
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("UP NEXT")
                            .size(9.5)
                            .strong()
                            .color(th::text_dim()),
                    );
                    if !upcoming.is_empty() {
                        ui.label(
                            RichText::new(format!("{} track(s)", upcoming.len()))
                                .size(9.5)
                                .color(th::text_dim()),
                        );
                    }
                });
                ui.add_space(2.0);

                if upcoming.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("{} Nothing queued", icons::CHECK.as_str()))
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                    ui.label(
                        RichText::new("Right-click a track and choose Play next or Add to queue.")
                            .size(10.0)
                            .color(th::text_dim()),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .id_salt("queue_list")
                    .auto_shrink([false, false])
                    .show_rows(ui, 26.0, upcoming.len(), |ui, range| {
                        let missing_ids: Vec<i64> = range
                            .clone()
                            .map(|row| upcoming[row].1)
                            .filter(|id| !local.contains_key(id))
                            .collect();
                        let fetched: HashMap<i64, Track> = if missing_ids.is_empty() {
                            HashMap::new()
                        } else {
                            app.lib
                                .tracks_by_ids(&missing_ids)
                                .unwrap_or_default()
                                .into_iter()
                                .map(|track| (track.id, track))
                                .collect()
                        };

                        let range_end = range.end;
                        for row in range {
                            let (index, id) = upcoming[row];
                            let display_row = row + 1;
                            let track = local
                                .get(&id)
                                .map(|track| (*track).clone())
                                .or_else(|| fetched.get(&id).cloned());
                            let Some(track) = track else {
                                ui.label(
                                    RichText::new(format!("{:>2}  missing track", index + 1))
                                        .size(10.5)
                                        .color(th::text_dim()),
                                );
                                continue;
                            };

                            let mut jump = false;
                            let mut remove = false;
                            let mut move_to = None;

                            ui.horizontal(|ui| {
                                let width = ui.available_width().max(40.0);
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(width, 23.0),
                                    egui::Sense::click_and_drag(),
                                );

                                if resp.hovered() {
                                    ui.painter().rect_filled(rect, 3.0, th::bg_row_alt());
                                }

                                // Fixed grid columns keep every title and artist at the
                                // same x, whatever the index width or text length.
                                let inner = rect.shrink2(egui::vec2(4.0, 0.0));
                                let index_width = 22.0;
                                let artist_width = (inner.width() * 0.34).clamp(52.0, 96.0);
                                let gap_width = 10.0;
                                let title_left = inner.left() + index_width + 4.0;
                                let artist_left = inner.right() - artist_width;
                                let title_width = (artist_left - gap_width - title_left).max(24.0);

                                let index_rect = egui::Rect::from_min_size(
                                    inner.min,
                                    egui::vec2(index_width, inner.height()),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(index_rect).layout(
                                        egui::Layout {
                                            main_align: egui::Align::Max,
                                            ..egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                    ),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(display_row.to_string())
                                                    .monospace()
                                                    .size(10.5)
                                                    .color(th::text_dim()),
                                            )
                                            .selectable(false),
                                        );
                                    },
                                );

                                let title_rect = egui::Rect::from_min_size(
                                    egui::pos2(title_left, inner.top()),
                                    egui::vec2(title_width, inner.height()),
                                );
                                let title_text = crop_to_width(ui, &track.title, title_width, 11.5);
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(title_rect).layout(
                                        egui::Layout {
                                            main_align: egui::Align::Min,
                                            ..egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                    ),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&title_text)
                                                    .size(11.5)
                                                    .color(th::text()),
                                            )
                                            .truncate()
                                            .selectable(false),
                                        );
                                    },
                                );

                                let artist_rect = egui::Rect::from_min_size(
                                    egui::pos2(artist_left, inner.top()),
                                    egui::vec2(artist_width, inner.height()),
                                );
                                let artist_text =
                                    crop_to_width(ui, &track.artist, artist_width, 11.0);
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(artist_rect).layout(
                                        egui::Layout {
                                            main_align: egui::Align::Max,
                                            ..egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                    ),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&artist_text)
                                                    .size(11.0)
                                                    .color(th::text_dim()),
                                            )
                                            .truncate()
                                            .selectable(false),
                                        );
                                    },
                                );

                                let resp = resp.on_hover_text(format!(
                                    "Play now\n{} - {}\nDrag to reorder",
                                    track.title,
                                    fmt_time(track.duration_secs as f64)
                                ));
                                resp.context_menu(|ui| {
                                    if ui.button(icons::with(icons::PLAY, "Play now")).clicked() {
                                        jump = true;
                                        ui.close();
                                    }
                                    if ui
                                        .button(icons::with(icons::X, "Remove from queue"))
                                        .clicked()
                                    {
                                        remove = true;
                                        ui.close();
                                    }
                                });
                                if resp.clicked() {
                                    jump = true;
                                }
                                if resp.drag_started() {
                                    egui::DragAndDrop::set_payload(
                                        &ctx,
                                        crate::app::QueueDrag { index },
                                    );
                                }
                                if let Some(drag) =
                                    resp.dnd_release_payload::<crate::app::QueueDrag>()
                                {
                                    move_to = Some(drag.index);
                                } else if resp
                                    .dnd_hover_payload::<crate::app::QueueDrag>()
                                    .is_some()
                                {
                                    let painter = ctx.layer_painter(egui::LayerId::new(
                                        egui::Order::Foreground,
                                        egui::Id::new("queue_dnd_insert"),
                                    ));
                                    painter.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(resp.rect.left(), resp.rect.top()),
                                            egui::pos2(resp.rect.right(), resp.rect.top() + 2.0),
                                        ),
                                        0.0,
                                        th::accent(),
                                    );
                                }
                            });

                            if jump {
                                act = Some(Act::Jump(index));
                            }
                            if remove {
                                act = Some(Act::Remove(index));
                            }
                            if let Some(from) = move_to {
                                act = Some(Act::Move { from, to: index });
                            }
                        }

                        // Drop below the last row to move an entry to the end.
                        if range_end == upcoming.len() {
                            let (end_rect, end_resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 18.0),
                                egui::Sense::hover(),
                            );
                            if end_resp
                                .dnd_hover_payload::<crate::app::QueueDrag>()
                                .is_some()
                            {
                                let painter = ctx.layer_painter(egui::LayerId::new(
                                    egui::Order::Foreground,
                                    egui::Id::new("queue_dnd_end"),
                                ));
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(end_rect.left(), end_rect.top()),
                                        egui::pos2(end_rect.right(), end_rect.top() + 2.0),
                                    ),
                                    0.0,
                                    th::accent(),
                                );
                            }
                            if let Some(drag) =
                                end_resp.dnd_release_payload::<crate::app::QueueDrag>()
                            {
                                act = Some(Act::Move {
                                    from: drag.index,
                                    to: queue.len(),
                                });
                            }
                        }
                    });
            });
    }

    match act {
        Some(Act::Jump(index)) => app.queue_jump(index),
        Some(Act::Remove(index)) => app.queue_remove(index),
        Some(Act::Move { from, to }) => app.queue_move(from, to),
        Some(Act::Clear) => {
            app.queue_clear_upcoming();
            app.message = "cleared the play queue".to_string();
        }
        Some(Act::Close) => {
            app.settings.queue_open = false;
            app.persist_settings();
        }
        None => {}
    }
}

pub fn tracklist(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    enum RowAct {
        Click {
            index: usize,
            ctrl: bool,
            shift: bool,
        },
        Play(usize),
        DragStart(usize),
        DropOn {
            index: usize,
            drag: crate::app::DragTracks,
        },
    }
    enum UiAction {
        Remove,
        MoveUp,
        MoveDown,
    }
    enum MenuAction {
        PlayNow(usize),
        PlayNext(usize),
        AddToQueue(usize),
        Rename(usize),
        EditTags(usize),
        FindMetadata(usize),
        RemoveFromPlaylist(usize),
        AddToPlaylist(usize, i64),
    }

    let mut act: Option<RowAct> = None;
    let mut ui_action: Option<UiAction> = None;
    let mut menu_action: Option<MenuAction> = None;
    let mut new_sort: Option<SortKey> = None;

    let tracks: &[Track] = &app.tracks;
    let selected = app.selected.clone();
    let playing_id = app.playing_id;
    let sort = app.sort;
    let sort_asc = app.sort_asc;
    let playlists = app.playlists.clone();
    let current_playlist = app.editable_playlist();
    let in_playlist = current_playlist.is_some();
    let mods = ctx.input(|i| i.modifiers);
    let (ctrl, shift) = (mods.ctrl || mods.command, mods.shift);

    let mut cell_focus: Option<TrackField> = None;

    let panel = egui::CentralPanel::default().show(ui, |ui| {
        if let Some(playlist_id) = app.editable_playlist() {
            let name = app
                .playlists
                .iter()
                .find(|p| p.id == playlist_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).size(12.0).strong().color(th::text()));
                ui.label(
                    RichText::new(format!("{} tracks", tracks.len()))
                        .size(11.0)
                        .color(th::text_dim()),
                );
                ui.separator();
                let has_selection = !app.selected.is_empty();
                if icons::button_enabled(ui, has_selection, icons::TRASH, "Remove")
                    .on_hover_text("Remove the selection from this playlist (Delete)")
                    .clicked()
                {
                    ui_action = Some(UiAction::Remove);
                }
                if icons::compact_enabled(ui, has_selection, icons::ARROW_UP, "Move up (Alt+Up)")
                    .clicked()
                {
                    ui_action = Some(UiAction::MoveUp);
                }
                if icons::compact_enabled(
                    ui,
                    has_selection,
                    icons::ARROW_DOWN,
                    "Move down (Alt+Down)",
                )
                .clicked()
                {
                    ui_action = Some(UiAction::MoveDown);
                }
                ui.label(
                    RichText::new("drag rows to reorder, drag onto another playlist to move")
                        .size(10.0)
                        .color(th::text_dim()),
                );
            });
            ui.separator();
        }

        if tracks.is_empty() {
            let hint = if !app.search.is_empty() {
                "No tracks match your search."
            } else if matches!(
                app.view,
                crate::app::View::Playlist(_) | crate::app::View::Smart(_)
            ) {
                "This view is empty. Drag tracks here from the library."
            } else {
                "Scan a music folder to build your library."
            };
            ui.add_space(70.0);
            ui.vertical_centered(|ui| {
                ui.label(icons::text(icons::MUSIC_NOTES, 42.0).color(th::text_dim()));
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Nothing here yet")
                        .size(16.0)
                        .strong()
                        .color(th::text()),
                );
                ui.add_space(2.0);
                ui.label(RichText::new(hint).size(11.0).color(th::text_dim()));
            });
            return;
        }

        // Clicks act on rows, never select text inside cells.
        ui.style_mut().interaction.selectable_labels = false;

        let table = TableBuilder::new(ui)
            .id_salt("tracks")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click_and_drag())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(44.0).clip(true))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::initial(150.0).at_least(70.0).clip(true))
            .column(Column::initial(170.0).at_least(70.0).clip(true))
            .column(Column::initial(110.0).at_least(60.0).clip(true))
            .column(Column::initial(52.0).at_least(36.0).clip(true))
            .column(Column::initial(52.0).at_least(36.0).clip(true))
            .column(Column::initial(78.0).at_least(50.0).clip(true));

        let mut header = |ui: &mut egui::Ui, label: &str, key: Option<SortKey>| {
            let active = key == Some(sort);
            let arrow = if active {
                if sort_asc {
                    format!(" {}", icons::CARET_UP.as_str())
                } else {
                    format!(" {}", icons::CARET_DOWN.as_str())
                }
            } else {
                String::new()
            };
            let text = RichText::new(format!("{label}{arrow}"))
                .size(11.0)
                .strong()
                .color(if active { th::accent() } else { th::text_dim() });
            let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            if resp.clicked() {
                if let Some(k) = key {
                    new_sort = Some(k);
                }
            }
        };

        table
            .header(24.0, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new("#").size(11.0).color(th::text_dim()));
                });
                h.col(|ui| header(ui, "Title", Some(SortKey::Title)));
                h.col(|ui| header(ui, "Artist", Some(SortKey::Artist)));
                h.col(|ui| header(ui, "Album", Some(SortKey::Album)));
                h.col(|ui| header(ui, "Genre", Some(SortKey::Genre)));
                h.col(|ui| header(ui, "Year", Some(SortKey::Year)));
                h.col(|ui| header(ui, "Time", Some(SortKey::Duration)));
                h.col(|ui| {
                    ui.label(RichText::new("Format").size(11.0).color(th::text_dim()));
                });
            })
            .body(|body| {
                body.rows(22.0, tracks.len(), |mut row| {
                    let idx = row.index();
                    let track = &tracks[idx];
                    let is_sel = selected.contains(&track.id);
                    let is_playing = playing_id == Some(track.id);
                    row.set_selected(is_sel);

                    let (_, cell) = row.col(|ui| {
                        if is_playing {
                            ui.label(icons::text(icons::PLAY, 11.0).color(th::accent()));
                        } else {
                            ui.label(
                                RichText::new(format!("{:>2}", track.track_no.unwrap_or(0)))
                                    .monospace()
                                    .size(11.0)
                                    .color(th::text_dim()),
                            );
                        }
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::TrackNo);
                    }

                    let (_, cell) = row.col(|ui| {
                        let text = RichText::new(&track.title).size(12.5);
                        ui.add(
                            egui::Label::new(if is_playing {
                                text.color(th::accent()).strong()
                            } else {
                                text
                            })
                            .truncate(),
                        );
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::Title);
                    }

                    let (_, cell) = row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&track.artist)
                                    .size(12.0)
                                    .color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::Artist);
                    }

                    let (_, cell) = row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&track.album).size(12.0).color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::Album);
                    }

                    let (_, cell) = row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&track.genre).size(11.5).color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::Genre);
                    }

                    let (_, cell) = row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    track.year.map(|y| y.to_string()).unwrap_or_default(),
                                )
                                .size(11.5)
                                .color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    });
                    if cell.clicked() {
                        cell_focus = Some(TrackField::Year);
                    }

                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(fmt_time(track.duration_secs as f64))
                                    .monospace()
                                    .size(11.5)
                                    .color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        let label = if track.bitrate > 0 {
                            format!("{}k", track.bitrate)
                        } else {
                            track.format.clone()
                        };
                        ui.add(
                            egui::Label::new(RichText::new(label).size(11.0).color(th::text_dim()))
                                .truncate(),
                        );
                    });

                    let resp = row.response();
                    if resp.secondary_clicked() && !is_sel {
                        act = Some(RowAct::Click {
                            index: idx,
                            ctrl: false,
                            shift: false,
                        });
                    }
                    resp.context_menu(|ui| {
                        if ui.button(icons::with(icons::PLAY, "Play now")).clicked() {
                            menu_action = Some(MenuAction::PlayNow(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::SKIP_FORWARD, "Play next"))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::PlayNext(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::QUEUE, "Add to queue"))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::AddToQueue(idx));
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(icons::with(icons::PENCIL_SIMPLE, "Rename"))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::Rename(idx));
                            ui.close();
                        }
                        if ui.button(icons::with(icons::TAG, "Edit tags")).clicked() {
                            menu_action = Some(MenuAction::EditTags(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::MAGIC_WAND, "Find metadata"))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::FindMetadata(idx));
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                in_playlist,
                                egui::Button::new(icons::with(
                                    icons::TRASH,
                                    "Remove from playlist",
                                )),
                            )
                            .on_disabled_hover_text("Only available inside a playlist")
                            .clicked()
                        {
                            menu_action = Some(MenuAction::RemoveFromPlaylist(idx));
                            ui.close();
                        }
                        ui.menu_button(icons::with(icons::PLUS, "Add to playlist"), |ui| {
                            if playlists.is_empty() {
                                ui.label(
                                    RichText::new("no playlists yet")
                                        .size(11.0)
                                        .color(th::text_dim()),
                                );
                            }
                            for playlist in &playlists {
                                if ui.button(&playlist.name).clicked() {
                                    menu_action = Some(MenuAction::AddToPlaylist(idx, playlist.id));
                                    ui.close();
                                }
                            }
                        });
                    });
                    if let Some(drag) = resp.dnd_release_payload::<crate::app::DragTracks>() {
                        act = Some(RowAct::DropOn {
                            index: idx,
                            drag: drag.as_ref().clone(),
                        });
                    } else if resp.dnd_hover_payload::<crate::app::DragTracks>().is_some() {
                        // Insertion marker: the row's own context, because the
                        // table already holds the panel's Ui.
                        let rect = resp.rect;
                        let painter = resp.ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("dnd_insert"),
                        ));
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.left(), rect.top()),
                                egui::pos2(rect.right(), rect.top() + 2.0),
                            ),
                            0.0,
                            th::accent(),
                        );
                    }
                    if resp.double_clicked() {
                        act = Some(RowAct::Play(idx));
                    } else if resp.clicked() {
                        act = Some(RowAct::Click {
                            index: idx,
                            ctrl,
                            shift,
                        });
                    } else if resp.drag_started() {
                        act = Some(RowAct::DragStart(idx));
                    }
                });
            });
    });

    // The track list itself is a drop target too: the open playlist, or the
    // library when browsing all music.
    app.os_drop_zones.push(crate::app::OsDropZone {
        playlist: current_playlist,
        rect: panel.response.rect,
    });
    if os_files_over(ui, panel.response.rect) {
        ui.painter().rect_stroke(
            panel.response.rect.shrink(1.0),
            0.0,
            egui::Stroke::new(1.5_f32, th::accent()),
            egui::StrokeKind::Inside,
        );
    }

    if let Some(field) = cell_focus {
        app.focus = Focus::Tracks;
        app.rename_field = field;
    }
    if act.is_some() || ui_action.is_some() {
        app.focus = Focus::Tracks;
    }

    if let Some(key) = new_sort {
        if app.sort == key {
            app.sort_asc = !app.sort_asc;
        } else {
            app.sort = key;
            app.sort_asc = true;
        }
        app.mark_tracks_dirty();
    }

    match act {
        Some(RowAct::Click { index, ctrl, shift }) => app.click_row(index, ctrl, shift),
        Some(RowAct::Play(index)) => app.play_from_list(index),
        Some(RowAct::DropOn { index, drag }) => app.drop_in_playlist(index, &drag),
        Some(RowAct::DragStart(index)) => {
            // Dragging an unselected row drags just that row.
            if let Some(track) = app.tracks.get(index) {
                if !app.selected.contains(&track.id) {
                    app.click_row(index, false, false);
                }
            }
            let ids = app.selected_in_view();
            if !ids.is_empty() {
                egui::DragAndDrop::set_payload(
                    &ctx,
                    crate::app::DragTracks {
                        ids,
                        from_playlist: app.editable_playlist(),
                    },
                );
            }
        }
        None => {}
    }

    if let Some(action) = ui_action {
        match action {
            UiAction::Remove => app.remove_selection_from_playlist(),
            UiAction::MoveUp => app.move_selection(-1),
            UiAction::MoveDown => app.move_selection(1),
        }
    }

    if let Some(action) = menu_action {
        app.focus = Focus::Tracks;
        match action {
            MenuAction::PlayNow(index) => {
                app.select_row_for_action(index);
                app.play_from_list(index);
            }
            MenuAction::PlayNext(index) => {
                app.select_row_for_action(index);
                app.queue_next_selected();
            }
            MenuAction::AddToQueue(index) => {
                app.select_row_for_action(index);
                app.queue_append_selected();
            }
            MenuAction::Rename(index) => {
                app.select_only_row(index);
                app.start_rename();
            }
            MenuAction::EditTags(index) => {
                app.select_only_row(index);
                app.open_editor();
            }
            MenuAction::FindMetadata(index) => {
                app.select_only_row(index);
                app.open_autotag();
            }
            MenuAction::RemoveFromPlaylist(index) => {
                app.select_row_for_action(index);
                app.remove_selection_from_playlist();
            }
            MenuAction::AddToPlaylist(index, playlist_id) => {
                app.select_row_for_action(index);
                app.add_to_playlist(playlist_id, &[]);
            }
        }
    }

    // List-local keys (not rebindable actions).
    if !ctx.egui_wants_keyboard_input() {
        if ctx.input(|i| i.key_pressed(egui::Key::A) && (i.modifiers.ctrl || i.modifiers.command)) {
            app.select_all_in_view();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.clear_selection();
        }
        if ctx.input(|i| {
            i.key_pressed(egui::Key::Delete) && !i.modifiers.ctrl && !i.modifiers.command
        }) {
            app.remove_selection_from_playlist();
        }
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowUp)) {
            app.move_selection(-1);
        }
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowDown)) {
            app.move_selection(1);
        }
    }
}

pub fn tag_editor(app: &mut App, ctx: &egui::Context) {
    let Some(state) = &mut app.editing else {
        return;
    };

    let mut save = false;
    let mut cancel = false;

    let batch = state.batch;
    let count = state.paths.len();
    let title = if batch {
        format!("Batch edit · {count} tracks")
    } else {
        "Edit tags".to_string()
    };
    let hint = if batch { "(unchanged)" } else { "" };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            if batch {
                ui.label(
                    RichText::new("Empty fields are left unchanged in every file.")
                        .size(11.0)
                        .color(th::accent()),
                );
            }
            ui.label(
                RichText::new(state.paths[0].display().to_string())
                    .size(10.0)
                    .color(th::text_dim()),
            );
            if batch {
                ui.label(
                    RichText::new(format!("…and {} more", count.saturating_sub(1)))
                        .size(10.0)
                        .color(th::text_dim()),
                );
            }
            ui.add_space(6.0);

            egui::Grid::new("tag_fields")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    let field = |ui: &mut egui::Ui, label: &str, value: &mut String| {
                        ui.label(RichText::new(label).size(12.0).color(th::text_dim()));
                        ui.add_sized(
                            [320.0, 22.0],
                            egui::TextEdit::singleline(value).hint_text(hint),
                        );
                        ui.end_row();
                    };
                    field(ui, "Title", &mut state.edit.title);
                    field(ui, "Artist", &mut state.edit.artist);
                    field(ui, "Album", &mut state.edit.album);
                    field(ui, "Album artist", &mut state.edit.album_artist);
                    field(ui, "Genre", &mut state.edit.genre);

                    let mut year = state.edit.year.unwrap_or(0);
                    let mut track_no = state.edit.track_no.unwrap_or(0);
                    let mut disc_no = state.edit.disc_no.unwrap_or(0);

                    ui.label(RichText::new("Year").size(12.0).color(th::text_dim()));
                    ui.add(egui::DragValue::new(&mut year).range(0..=9999).speed(1.0));
                    ui.end_row();

                    ui.label(RichText::new("Track no").size(12.0).color(th::text_dim()));
                    ui.add(
                        egui::DragValue::new(&mut track_no)
                            .range(0..=999)
                            .speed(1.0),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Disc no").size(12.0).color(th::text_dim()));
                    ui.add(egui::DragValue::new(&mut disc_no).range(0..=99).speed(1.0));
                    ui.end_row();

                    state.edit.year = (year > 0).then_some(year);
                    state.edit.track_no = (track_no > 0).then_some(track_no);
                    state.edit.disc_no = (disc_no > 0).then_some(disc_no);

                    if batch {
                        ui.label(
                            RichText::new("0 keeps the existing number")
                                .size(9.0)
                                .color(th::text_dim()),
                        );
                        ui.end_row();
                    }
                });

            if let Some(status) = &state.status {
                ui.add_space(4.0);
                ui.label(RichText::new(status).size(11.0).color(th::accent()));
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let save_label = if batch {
                    format!("Save to {count} files")
                } else {
                    "Save".to_string()
                };
                if ui.button(save_label).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                ui.label(
                    RichText::new(if batch {
                        "Only filled fields are written"
                    } else {
                        "Writes tags into the file itself"
                    })
                    .size(10.0)
                    .color(th::text_dim()),
                );
            });
        });

    if save {
        app.save_editor();
    } else if cancel {
        app.editing = None;
    }
}

/// F2 rename dialog: one text field, shared by track tags and playlists.
pub fn rename_window(app: &mut App, ctx: &egui::Context) {
    let Some(state) = &mut app.rename else {
        return;
    };

    let mut commit = false;
    let mut cancel = false;
    let title = icons::with(icons::PENCIL_SIMPLE, &state.target.title());

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let hint = match &state.target {
                RenameTarget::Track { field, .. } => {
                    format!("New {}", field.label().to_lowercase())
                }
                _ => "New name".to_string(),
            };
            ui.label(RichText::new(hint).size(10.5).color(th::text_dim()));
            ui.add_space(4.0);

            let resp = ui.add_sized(
                [300.0, 24.0],
                egui::TextEdit::singleline(&mut state.text).hint_text("new value"),
            );
            if state.request_focus {
                resp.request_focus();
                state.request_focus = false;
            }

            if let Some(error) = &state.error {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} {error}", icons::WARNING.as_str()))
                        .size(10.5)
                        .color(Color32::from_rgb(230, 120, 120)),
                );
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if icons::button(ui, icons::CHECK, "Save").clicked() {
                    commit = true;
                }
                if icons::button(ui, icons::X, "Cancel").clicked() {
                    cancel = true;
                }
                ui.label(
                    RichText::new("Enter saves, Escape cancels")
                        .size(10.0)
                        .color(th::text_dim()),
                );
            });

            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });

    if commit {
        app.commit_rename();
    } else if cancel {
        app.rename = None;
    }
}

pub fn playlist_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut name) = app.playlist_dialog.clone() else {
        return;
    };

    let mut create = false;
    let mut cancel = false;
    egui::Window::new("New playlist")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let selected = app.selected_in_view().len();
            let total = if selected > 0 {
                selected
            } else {
                app.tracks.len()
            };
            ui.label(
                RichText::new(if selected > 0 {
                    format!("{selected} selected track(s) will be saved")
                } else {
                    format!("all {total} tracks in this view will be saved")
                })
                .size(11.0)
                .color(th::text_dim()),
            );
            ui.add_space(4.0);
            let resp = ui.add_sized(
                [260.0, 22.0],
                egui::TextEdit::singleline(&mut name).hint_text("playlist name"),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                create = true;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    create = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

    app.playlist_dialog = Some(name);
    if create {
        app.commit_playlist_dialog();
    } else if cancel {
        app.playlist_dialog = None;
    }
}

/// One selectable track row in the hygiene window: metadata on the first line,
/// full path on the second.
fn hygiene_row(ui: &mut egui::Ui, selected: &mut BTreeSet<i64>, track: &Track) {
    ui.horizontal(|ui| {
        let mut checked = selected.contains(&track.id);
        if ui.checkbox(&mut checked, "").changed() {
            if checked {
                selected.insert(track.id);
            } else {
                selected.remove(&track.id);
            }
        }
        let mut label = if track.title.is_empty() {
            "(untitled)".to_string()
        } else {
            track.title.clone()
        };
        if !track.artist.is_empty() {
            label.push_str("  -  ");
            label.push_str(&track.artist);
        }
        ui.add(egui::Label::new(RichText::new(label).size(11.5).color(th::text())).truncate());
    });
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        ui.add(
            egui::Label::new(RichText::new(&track.path).size(9.5).color(th::text_dim())).truncate(),
        )
        .on_hover_text(&track.path);
    });
}

pub fn hygiene_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_hygiene {
        return;
    }
    let Some(state) = app.hygiene.as_mut() else {
        app.show_hygiene = false;
        return;
    };

    enum Act {
        Rescan,
        SelectMissing,
        SelectDuplicates,
        Clear,
        Ask(Vec<i64>),
        Confirm,
        Cancel,
    }
    let mut act: Option<Act> = None;
    let mut open = app.show_hygiene;
    let duplicate_tracks: usize = state.duplicate_groups.iter().map(|group| group.len()).sum();

    egui::Window::new("Library hygiene")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .min_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button(icons::with(icons::ARROWS_CLOCKWISE, "Rescan"))
                    .clicked()
                {
                    act = Some(Act::Rescan);
                }
                ui.label(
                    RichText::new(format!(
                        "{} missing · {} duplicate group(s) · {} duplicate track(s)",
                        state.missing.len(),
                        state.duplicate_groups.len(),
                        duplicate_tracks
                    ))
                    .size(11.0)
                    .color(th::text_dim()),
                );
            });
            ui.label(
                RichText::new("Removing entries never deletes audio files from disk.")
                    .size(10.5)
                    .color(th::text_dim()),
            );
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("hygiene_scroll")
                .max_height(420.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::CollapsingHeader::new(format!("Missing files ({})", state.missing.len()))
                        .default_open(true)
                        .show(ui, |ui| {
                            if state.missing.is_empty() {
                                ui.label(
                                    RichText::new("Every indexed file still exists.")
                                        .size(11.0)
                                        .color(th::text_dim()),
                                );
                            } else {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Select all").clicked() {
                                        act = Some(Act::SelectMissing);
                                    }
                                    if ui.small_button("Prune all").clicked() {
                                        act = Some(Act::Ask(
                                            state.missing.iter().map(|track| track.id).collect(),
                                        ));
                                    }
                                });
                                for track in &state.missing {
                                    hygiene_row(ui, &mut state.selected, track);
                                }
                            }
                        });

                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(format!(
                        "Duplicates ({} groups)",
                        state.duplicate_groups.len()
                    ))
                    .default_open(true)
                    .show(ui, |ui| {
                        if state.duplicate_groups.is_empty() {
                            ui.label(
                                RichText::new("No tracks share the same artist, title and album.")
                                    .size(11.0)
                                    .color(th::text_dim()),
                            );
                        } else {
                            if ui
                                .small_button("Select all but the first in each group")
                                .clicked()
                            {
                                act = Some(Act::SelectDuplicates);
                            }
                            for group in &state.duplicate_groups {
                                ui.add_space(6.0);
                                let first = &group[0];
                                let mut heading = format!(
                                    "{}  -  {}",
                                    if first.title.is_empty() {
                                        "(untitled)"
                                    } else {
                                        first.title.as_str()
                                    },
                                    first.artist
                                );
                                if !first.album.is_empty() {
                                    heading.push_str("  ·  ");
                                    heading.push_str(&first.album);
                                }
                                ui.label(
                                    RichText::new(heading).size(11.5).strong().color(th::text()),
                                );
                                for track in group {
                                    hygiene_row(ui, &mut state.selected, track);
                                }
                            }
                        }
                    });
                });

            ui.separator();
            if let Some(ids) = &state.pending {
                let count = ids.len();
                ui.label(
                    RichText::new(format!("Remove {count} track(s) from the library?"))
                        .size(12.0)
                        .strong()
                        .color(th::accent()),
                );
                ui.label(
                    RichText::new("The audio files stay on disk and can be re-imported later.")
                        .size(10.5)
                        .color(th::text_dim()),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(icons::with(icons::TRASH, "Remove from library"))
                        .clicked()
                    {
                        act = Some(Act::Confirm);
                    }
                    if ui.button("Cancel").clicked() {
                        act = Some(Act::Cancel);
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} selected", state.selected.len()))
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                    let label = format!("Remove {} from library", state.selected.len());
                    if ui
                        .add_enabled(
                            !state.selected.is_empty(),
                            egui::Button::new(icons::with(icons::TRASH, &label)),
                        )
                        .on_hover_text("Deletes library entries only; files stay on disk")
                        .clicked()
                    {
                        act = Some(Act::Ask(state.selected.iter().copied().collect()));
                    }
                    if ui.small_button("Clear selection").clicked() && !state.selected.is_empty() {
                        act = Some(Act::Clear);
                    }
                });
            }
            if let Some(status) = &state.status {
                ui.label(RichText::new(status).size(10.5).color(th::accent()));
            }
        });

    app.show_hygiene = open;
    match act {
        Some(Act::Rescan) => app.scan_hygiene(),
        Some(Act::SelectMissing) => {
            if let Some(state) = &mut app.hygiene {
                state.selected = state.missing.iter().map(|track| track.id).collect();
            }
        }
        Some(Act::SelectDuplicates) => {
            if let Some(state) = &mut app.hygiene {
                for group in &state.duplicate_groups {
                    state
                        .selected
                        .extend(group.iter().skip(1).map(|track| track.id));
                }
            }
        }
        Some(Act::Clear) => {
            if let Some(state) = &mut app.hygiene {
                state.selected.clear();
            }
        }
        Some(Act::Ask(ids)) => {
            if let Some(state) = &mut app.hygiene {
                state.pending = Some(ids);
            }
        }
        Some(Act::Confirm) => {
            let ids = app
                .hygiene
                .as_mut()
                .and_then(|state| state.pending.take())
                .unwrap_or_default();
            app.remove_from_library(&ids);
        }
        Some(Act::Cancel) => {
            if let Some(state) = &mut app.hygiene {
                state.pending = None;
            }
        }
        None => {}
    }
}

pub fn settings_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }

    let mut open = true;
    let mut reset = false;
    let mut unbind: Option<crate::settings::Action> = None;
    let mut rebind: Option<crate::settings::Action> = None;
    let mut chosen_skin: Option<crate::theme::Skin> = None;
    let conflicts = app.settings.conflicts();
    let mut tab = app.settings_tab;

    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, SettingsTab::General, "General");
                ui.selectable_value(&mut tab, SettingsTab::Keybindings, "Keybindings");
                ui.selectable_value(&mut tab, SettingsTab::About, "About");
            });
            ui.separator();
            ui.add_space(4.0);

            if tab == SettingsTab::General {
                ui.label(
                    RichText::new("SKIN")
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt("skin_select")
                    .selected_text(RichText::new(app.settings.skin.label()).size(12.0))
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for skin in crate::theme::Skin::ALL {
                            if ui
                                .selectable_label(app.settings.skin == skin, skin.label())
                                .clicked()
                            {
                                chosen_skin = Some(skin);
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Skins apply immediately and are remembered in settings.json.")
                        .size(10.5)
                        .color(th::text_dim()),
                );

                ui.add_space(8.0);
                let mut watching = app.settings.watch_enabled;
                if ui
                    .checkbox(
                        &mut watching,
                        RichText::new("Watch scanned folders for new or changed files").size(11.5),
                    )
                    .on_hover_text(
                        "New audio files are imported automatically and tag edits refresh \
                         existing rows. Manual Scan still works.",
                    )
                    .changed()
                {
                    app.set_watch_enabled(watching);
                }
                if app.settings.watch_folders.is_empty() {
                    ui.label(
                        RichText::new("Scan a folder once to start watching it.")
                            .size(10.0)
                            .color(th::text_dim()),
                    );
                } else {
                    let folders = app.settings.watch_folders.join("\n");
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} watched folder(s)",
                                app.settings.watch_folders.len()
                            ))
                            .size(10.0)
                            .color(th::text_dim()),
                        )
                        .on_hover_text(folders);
                        if ui
                            .small_button("Forget")
                            .on_hover_text("Stop watching these folders")
                            .clicked()
                        {
                            app.forget_watch_folders();
                        }
                    });
                }
            }

            if tab == SettingsTab::Keybindings {
                ui.label(
                    RichText::new("KEYBOARD SHORTCUTS")
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(
                        "Click a binding, then press the new key combination. \
                         Delete clears it, Escape cancels.",
                    )
                    .size(11.0)
                    .color(th::text_dim()),
                );
                ui.add_space(8.0);

                egui::Grid::new("shortcut_grid")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for action in crate::settings::Action::ALL {
                            ui.label(RichText::new(action.label()).size(12.0));

                            let capturing = app.capture == Some(action);
                            let bound = app.settings.shortcut(action);
                            let text = if capturing {
                                "press a key…".to_string()
                            } else {
                                bound
                                    .map(|s| s.label())
                                    .unwrap_or_else(|| "unbound".to_string())
                            };

                            let conflict = conflicts.contains(&action);
                            let mut button = egui::Button::new(
                                RichText::new(text).size(12.0).color(if capturing {
                                    th::accent()
                                } else if conflict {
                                    Color32::from_rgb(230, 120, 120)
                                } else {
                                    th::text()
                                }),
                            );
                            if capturing {
                                button = button.fill(th::bg_selected());
                            }

                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized([190.0, 22.0], button)
                                    .on_hover_text(if conflict {
                                        "This combination is also used by another action"
                                    } else {
                                        "Click to rebind"
                                    })
                                    .clicked()
                                {
                                    rebind = Some(action);
                                }
                                if bound.is_some()
                                    && ui
                                        .small_button("x")
                                        .on_hover_text("Clear this binding")
                                        .clicked()
                                {
                                    unbind = Some(action);
                                }
                            });
                            ui.end_row();
                        }
                    });

                if !conflicts.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Conflicting bindings are shown in red.")
                            .size(10.0)
                            .color(Color32::from_rgb(230, 120, 120)),
                    );
                }
            }

            if tab == SettingsTab::About {
                ui.label(
                    RichText::new("MusicWasp")
                        .size(20.0)
                        .strong()
                        .color(th::accent()),
                );
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(th::text_dim()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "A MusicBee-inspired music library and player for Linux and Windows.",
                    )
                    .size(12.0),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Website").size(11.0).color(th::text_dim()));
                    ui.hyperlink_to("https://giwi.fr", "https://giwi.fr");
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Copyright © 2026 giwi.fr")
                        .size(11.5)
                        .color(th::text()),
                );
                ui.label(
                    RichText::new("Released under the GNU General Public License v3.")
                        .size(11.0)
                        .color(th::text_dim()),
                );
                ui.hyperlink_to(
                    "GPL-3.0-only license text",
                    "https://www.gnu.org/licenses/gpl-3.0.html",
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Settings file")
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.label(
                    RichText::new(app.settings_path.display().to_string())
                        .size(10.0)
                        .color(th::text_dim()),
                );
            }

            if tab == SettingsTab::General {
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").clicked() {
                        reset = true;
                    }
                    ui.label(
                        RichText::new("Themes, watching, shortcuts and session are reset.")
                            .size(10.0)
                            .color(th::text_dim()),
                    );
                });
            }
        });

    app.settings_tab = tab;

    if app.captured_this_frame {
        rebind = None;
    }
    app.captured_this_frame = false;

    if let Some(skin) = chosen_skin {
        app.set_skin(ctx, skin);
    }
    if let Some(action) = rebind {
        app.capture = Some(action);
    }
    if let Some(action) = unbind {
        app.settings.unbind(action);
        app.capture = None;
        app.persist_settings();
    }
    if reset {
        app.reset_settings(ctx);
    }
    if !open {
        app.show_settings = false;
        app.capture = None;
    }
}

pub fn folder_picker(app: &mut App, ctx: &egui::Context) {
    let Some(picker) = &mut app.picker else {
        return;
    };

    let mut open = true;
    let mut navigate: Option<PathBuf> = None;
    let mut choose = false;
    let mut toggle_hidden = false;

    egui::Window::new("Choose music folder")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .default_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Up").clicked() {
                    if let Some(parent) = picker.current.parent().map(PathBuf::from) {
                        navigate = Some(parent);
                    }
                }
                if ui.button("Home").clicked() {
                    if let Some(home) = dirs::home_dir() {
                        navigate = Some(home);
                    }
                }
                if ui
                    .checkbox(&mut picker.show_hidden, "Hidden folders")
                    .changed()
                {
                    toggle_hidden = true;
                }
            });

            ui.add_space(4.0);
            ui.label(
                RichText::new(picker.current.display().to_string())
                    .monospace()
                    .size(11.0)
                    .color(th::text_dim()),
            );
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(280.0)
                .show(ui, |ui| {
                    for dir in &picker.entries {
                        let name = dir
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let label = format!("📁 {name}");
                        let resp = ui.selectable_label(false, label);
                        if resp.clicked() {
                            navigate = Some(dir.clone());
                        }
                        if resp.double_clicked() {
                            navigate = Some(dir.clone());
                        }
                    }
                    if picker.entries.is_empty() {
                        ui.label(
                            RichText::new("no sub-folders")
                                .size(11.0)
                                .color(th::text_dim()),
                        );
                    }
                    if let Some(error) = &picker.error {
                        ui.label(
                            RichText::new(error)
                                .size(10.0)
                                .color(Color32::from_rgb(220, 110, 110)),
                        );
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button("Use this folder")
                    .on_hover_text("Scan this folder")
                    .clicked()
                {
                    choose = true;
                }
                ui.label(
                    RichText::new("Music folders are scanned recursively")
                        .size(10.0)
                        .color(th::text_dim()),
                );
            });
        });

    if toggle_hidden {
        if let Some(picker) = &mut app.picker {
            picker.reload();
        }
    }
    if let Some(dir) = navigate {
        if let Some(picker) = &mut app.picker {
            picker.navigate(dir);
        }
    }
    if choose {
        if let Some(picker) = &app.picker {
            app.scan_path = picker.current.to_string_lossy().to_string();
        }
        app.picker = None;
        app.scan_clicked();
        return;
    }
    if !open {
        app.picker = None;
    }
}

pub fn autotag_window(app: &mut App, ctx: &egui::Context) {
    use crate::app::AutoTagStage;

    let Some(state) = &mut app.autotag else {
        return;
    };
    let targets = state.targets.len();

    let mut open = true;
    let mut search = false;
    let mut apply = false;
    let mut reload_release = false;

    egui::Window::new("Auto-tag by album")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(960.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Artist").size(12.0).color(th::text_dim()));
                ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(&mut state.artist));
                ui.label(RichText::new("Album").size(12.0).color(th::text_dim()));
                ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(&mut state.album));
                if ui
                    .add_enabled(
                        !matches!(state.stage, AutoTagStage::Searching | AutoTagStage::Applying),
                        egui::Button::new("Search"),
                    )
                    .clicked()
                {
                    search = true;
                }
            });

            ui.add_space(6.0);

            match &state.stage {
                AutoTagStage::Editing => {
                    ui.label(
                        RichText::new(format!(
                            "{targets} track(s) will be tagged. MusicBrainz + Cover Art Archive, no API key needed."
                        ))
                        .size(11.0)
                        .color(th::text_dim()),
                    );
                }
                AutoTagStage::Searching => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Searching MusicBrainz…").size(12.0));
                    });
                }
                AutoTagStage::Applying => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Writing tags and cover…").size(12.0));
                    });
                }
                AutoTagStage::Results(candidates) => {
                    if candidates.is_empty() {
                        ui.label(
                            RichText::new("No releases found. Try a shorter album name.")
                                .size(12.0)
                                .color(th::text_dim()),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!("{} candidate release(s)", candidates.len()))
                                .size(11.0)
                                .color(th::text_dim()),
                        );
                        ui.add_space(2.0);

                        egui::ScrollArea::vertical()
                            .id_salt("autotag_results")
                            .max_height(200.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (index, candidate) in candidates.iter().enumerate() {
                                    let selected = state.selected == index;
                                    let label = format!(
                                        "{}  ·  {}  ·  {}  ·  {} tracks  ·  score {}",
                                        candidate.title,
                                        candidate.artist,
                                        candidate.date.clone().unwrap_or_else(|| "year?".into()),
                                        candidate.track_count,
                                        candidate.score
                                    );
                                    if ui.selectable_label(selected, label).clicked() {
                                        state.selected = index;
                                        reload_release = true;
                                    }
                                }
                            });

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);
                        if state.loading_release {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new("Loading the release's track list…")
                                        .size(11.0)
                                        .color(th::text_dim()),
                                );
                            });
                        } else if let Some(error) = &state.release_error {
                            ui.label(
                                RichText::new(format!(
                                    "{} could not load track list: {error}",
                                    icons::WARNING.as_str()
                                ))
                                .size(10.5)
                                .color(Color32::from_rgb(230, 120, 120)),
                            );
                        } else if let Some(release) = state.release.clone() {
                            ui.label(
                                RichText::new(
                                    "Review the file-to-release mapping before writing. \
                                     Pick another track in any row to override it.",
                                )
                                .size(10.5)
                                .color(th::text_dim()),
                            );
                            ui.add_space(4.0);

                            let mut seen = BTreeSet::new();
                            let mut duplicates = 0usize;
                            for mapping in &state.mappings {
                                if let Some(index) = mapping.release_index {
                                    if !seen.insert(index) {
                                        duplicates += 1;
                                    }
                                }
                            }

                            egui::ScrollArea::vertical()
                                .id_salt("autotag_mapping")
                                .max_height(280.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    egui::Grid::new("autotag_mapping_grid")
                                        .num_columns(3)
                                        .spacing([12.0, 6.0])
                                        .striped(true)
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new("File")
                                                    .size(10.0)
                                                    .strong()
                                                    .color(th::text_dim()),
                                            );
                                            ui.label(
                                                RichText::new("Release track")
                                                    .size(10.0)
                                                    .strong()
                                                    .color(th::text_dim()),
                                            );
                                            ui.label(
                                                RichText::new("Changes")
                                                    .size(10.0)
                                                    .strong()
                                                    .color(th::text_dim()),
                                            );
                                            ui.end_row();

                                            for mapping in &mut state.mappings {
                                                let file_label = if mapping.track.title.is_empty()
                                                {
                                                    PathBuf::from(&mapping.track.path)
                                                        .file_name()
                                                        .map(|name| name.to_string_lossy().to_string())
                                                        .unwrap_or_else(|| mapping.track.path.clone())
                                                } else {
                                                    format!(
                                                        "{:02}. {}  -  {}",
                                                        mapping.track.track_no.unwrap_or(0),
                                                        mapping.track.title,
                                                        mapping.track.artist
                                                    )
                                                };
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(file_label)
                                                            .size(11.0)
                                                            .color(th::text()),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(&mapping.track.path);

                                                let selected = mapping
                                                    .release_index
                                                    .and_then(|index| release.tracks.get(index))
                                                    .map(|track| {
                                                        format!(
                                                            "{}.{:02}  {}",
                                                            track.disc, track.number, track.title
                                                        )
                                                    })
                                                    .unwrap_or_else(|| "— skip —".to_string());
                                                egui::ComboBox::from_id_salt((
                                                    "autotag_map",
                                                    mapping.track.id,
                                                ))
                                                .selected_text(RichText::new(selected).size(11.0))
                                                .width(240.0)
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut mapping.release_index,
                                                        None,
                                                        "— skip —",
                                                    );
                                                    for (index, track) in
                                                        release.tracks.iter().enumerate()
                                                    {
                                                        let label = format!(
                                                            "{}.{:02}  {}",
                                                            track.disc,
                                                            track.number,
                                                            track.title
                                                        );
                                                        ui.selectable_value(
                                                            &mut mapping.release_index,
                                                            Some(index),
                                                            label,
                                                        );
                                                    }
                                                });

                                                let changes = mapping.changes(&release);
                                                let summary = if changes.is_empty() {
                                                    "no changes".to_string()
                                                } else {
                                                    changes
                                                        .iter()
                                                        .map(|(field, old, new)| {
                                                            format!("{field}: {old} → {new}")
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .join(" · ")
                                                };
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(summary).size(10.5).color(
                                                            if changes.is_empty() {
                                                                th::text_dim()
                                                            } else {
                                                                th::accent()
                                                            },
                                                        ),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(
                                                    changes
                                                        .iter()
                                                        .map(|(field, old, new)| {
                                                            format!("{field}: {old} → {new}")
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .join("\n"),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                });

                            let skipped = state
                                .mappings
                                .iter()
                                .filter(|mapping| mapping.release_index.is_none())
                                .count();
                            if duplicates > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "{} row(s) use the same release track.",
                                        duplicates
                                    ))
                                    .size(10.5)
                                    .color(Color32::from_rgb(230, 150, 90)),
                                );
                            }
                            if skipped > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "{skipped} file(s) will be skipped."
                                    ))
                                    .size(10.5)
                                    .color(th::text_dim()),
                                );
                            }
                            ui.label(
                                RichText::new(format!(
                                    "Album: {}  ·  {}",
                                    release.title, release.album_artist
                                ))
                                .size(10.0)
                                .color(th::text_dim()),
                            );
                        }

                        ui.add_space(4.0);
                        let mapped = state
                            .mappings
                            .iter()
                            .filter(|mapping| mapping.release_index.is_some())
                            .count();
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.embed_art, "Embed front cover");
                            let can_apply =
                                mapped > 0 && state.release.is_some() && !state.loading_release;
                            let response = ui
                                .add_enabled(
                                    can_apply,
                                    egui::Button::new(format!("Apply to {mapped} track(s)")),
                                )
                                .on_disabled_hover_text(
                                    "Wait for the release track list and map at least one file",
                                );
                            if response.clicked() {
                                apply = true;
                            }
                        });
                        if let Some(candidate) = candidates.get(state.selected) {
                            ui.label(
                                RichText::new(format!("release id: {}", candidate.release_id))
                                    .size(9.5)
                                    .color(th::text_dim()),
                            );
                        }
                    }
                }
                AutoTagStage::Done(message) => {
                    let failed = message.starts_with("failed") || message.contains("failed");
                    ui.label(
                        RichText::new(message)
                            .size(12.0)
                            .color(if failed {
                                Color32::from_rgb(230, 120, 120)
                            } else {
                                th::accent()
                            }),
                    );
                    if ui.button("Search again").clicked() {
                        search = true;
                    }
                }
            }
        });

    if search {
        app.start_autotag_search();
    }
    if reload_release {
        app.load_autotag_release();
    }
    if apply {
        app.start_autotag_apply();
    }
    if !open {
        app.autotag = None;
    }
}

pub fn smart_dialog(app: &mut App, ctx: &egui::Context) {
    use musicwasp_core::smart::{Field, Op, SortBy};

    let Some(draft) = &mut app.smart_dialog else {
        return;
    };

    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    let mut remove_rule: Option<usize> = None;

    egui::Window::new("New smart playlist")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Name").size(12.0).color(th::text_dim()));
                ui.add_sized([240.0, 22.0], egui::TextEdit::singleline(&mut draft.name));
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Match").size(12.0).color(th::text_dim()));
                egui::ComboBox::from_id_salt("match_mode")
                    .selected_text(if draft.definition.match_all {
                        "all rules"
                    } else {
                        "any rule"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.definition.match_all, true, "all rules");
                        ui.selectable_value(&mut draft.definition.match_all, false, "any rule");
                    });
                if ui.button("Add rule").clicked() {
                    draft
                        .definition
                        .rules
                        .push(musicwasp_core::smart::Rule::new(Field::Genre, Op::Contains));
                }
            });

            ui.add_space(4.0);
            egui::Grid::new("smart_rules")
                .num_columns(4)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for (index, rule) in draft.definition.rules.iter_mut().enumerate() {
                        egui::ComboBox::from_id_salt(("field", index))
                            .width(110.0)
                            .selected_text(rule.field.label())
                            .show_ui(ui, |ui| {
                                for field in Field::ALL {
                                    ui.selectable_value(&mut rule.field, field, field.label());
                                }
                            });

                        egui::ComboBox::from_id_salt(("op", index))
                            .width(110.0)
                            .selected_text(rule.op.label())
                            .show_ui(ui, |ui| {
                                for op in Op::ALL {
                                    ui.selectable_value(&mut rule.op, op, op.label());
                                }
                            });

                        ui.add_sized(
                            [150.0, 22.0],
                            egui::TextEdit::singleline(&mut rule.value).hint_text("value"),
                        );

                        if ui.small_button("x").clicked() {
                            remove_rule = Some(index);
                        }
                        ui.end_row();
                    }
                });

            if draft.definition.rules.is_empty() {
                ui.label(
                    RichText::new("No rules: the playlist contains everything.")
                        .size(10.0)
                        .color(th::text_dim()),
                );
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Sort").size(12.0).color(th::text_dim()));
                egui::ComboBox::from_id_salt("smart_sort")
                    .selected_text(draft.definition.sort.label())
                    .show_ui(ui, |ui| {
                        for sort in SortBy::ALL {
                            ui.selectable_value(&mut draft.definition.sort, sort, sort.label());
                        }
                    });

                let mut limited = draft.definition.limit.is_some();
                if ui.checkbox(&mut limited, "Limit to").changed() {
                    draft.definition.limit = limited.then_some(100);
                }
                if let Some(limit) = &mut draft.definition.limit {
                    ui.add(egui::DragValue::new(limit).range(1..=10_000).speed(1.0));
                    ui.label(RichText::new("tracks").size(10.0).color(th::text_dim()));
                }
            });

            ui.add_space(4.0);
            let preview = app
                .lib
                .evaluate_smart(&draft.definition)
                .map(|t| t.len())
                .unwrap_or(0);
            ui.label(
                RichText::new(format!("matches {preview} track(s) right now"))
                    .size(11.0)
                    .color(th::accent()),
            );

            if let Some(status) = &draft.status {
                ui.label(
                    RichText::new(status)
                        .size(11.0)
                        .color(Color32::from_rgb(230, 120, 120)),
                );
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    create = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                ui.label(
                    RichText::new("Smart playlists re-query the library every time they open")
                        .size(10.0)
                        .color(th::text_dim()),
                );
            });
        });

    if let Some(index) = remove_rule {
        if let Some(draft) = &mut app.smart_dialog {
            if index < draft.definition.rules.len() {
                draft.definition.rules.remove(index);
            }
        }
    }
    if create {
        app.commit_smart_dialog();
    } else if cancel {
        app.smart_dialog = None;
    }
    if !open {
        app.smart_dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::crop_with_hyphen;

    fn char_width(text: &str) -> f32 {
        text.chars().count() as f32
    }

    #[test]
    fn cropping_marks_the_cut_with_a_hyphen() {
        let measure = |text: &str| char_width(text);
        assert_eq!(crop_with_hyphen("Short", 10.0, 1.0, &measure), "Short");
        assert_eq!(
            crop_with_hyphen("A long title", 6.0, 1.0, &measure),
            "A lon-"
        );
        // Trailing spaces are dropped before the hyphen.
        assert_eq!(crop_with_hyphen("Two words", 4.0, 1.0, &measure), "Two-");
    }
}
