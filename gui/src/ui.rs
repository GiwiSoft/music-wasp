use crate::app::{
    fmt_size, fmt_time, App, BatchAction, BatchCase, BatchField, DragPlaylist, DragTracks, Focus,
    M3uMode, RenameTarget, SettingsTab, StreamKind, TrackField, View,
};
use crate::i18n::{tr, Language};
use crate::icons;
use crate::settings::{TrackColumn, TrackLayout};
use crate::theme as th;
use egui::{Color32, RichText, WidgetInfo, WidgetType};
use egui_extras::{Column, TableBuilder};
use musicwasp_core::smart::SmartPlaylist;
use musicwasp_core::{Playlist, PlaylistFolder, SortKey, Track};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

/// True while files are being dragged from the OS and the pointer is over `rect`.
fn os_files_over(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.ctx().input(|i| {
        !i.raw.hovered_files.is_empty() && i.pointer.hover_pos().is_some_and(|p| rect.contains(p))
    })
}

/// "Artist  ·  Album", or just the artist when the album tag is empty.
fn artist_album_line(track: &Track) -> String {
    if track.album.is_empty() {
        track.artist.clone()
    } else {
        format!("{}  ·  {}", track.artist, track.album)
    }
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
    let response = ui.add(
        egui::Button::new(icons::text(icon, 11.0).color(th::text_dim()))
            .frame(false)
            .min_size(egui::vec2(18.0, 18.0)),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, tooltip));
    response.on_hover_text(tooltip).clicked()
}

/// Section title; clicking it also folds the section.
fn section_label(ui: &mut egui::Ui, icon: egui_phosphor_icons::Icon, title: &str) -> bool {
    let text = th::semibold(format!("{} {}", icon.as_str(), title), 10.5).color(th::text_dim());
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A full-width navigation row with a pill highlight, used by the browser.
/// `indent` shifts the label for nested playlists.
fn nav_row(
    ui: &mut egui::Ui,
    selected: bool,
    indent: f32,
    text: impl Into<String>,
) -> egui::Response {
    let label = text.into();
    let height = if th::is_compact() { 20.0 } else { 26.0 };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.0), height),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &label,
        )
    });

    if selected {
        ui.painter()
            .rect_filled(rect, th::corner(7), ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, th::corner(7), th::hover_fill());
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0, rect.center().y - 6.5),
            egui::vec2(3.0, 13.0),
        );
        ui.painter().rect_filled(bar, th::pill(), th::accent());
    }

    let color = if selected { th::accent() } else { th::text() };
    let available = (rect.width() - indent - 22.0).max(12.0);
    let rich = RichText::new(&label)
        .size(if th::is_compact() { 12.0 } else { 13.0 })
        .color(color);
    let galley = egui::WidgetText::from(rich).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        available,
        egui::TextStyle::Button,
    );
    let pos = egui::pos2(
        rect.left() + 12.0 + indent,
        rect.center().y - galley.size().y * 0.5,
    );
    ui.painter().galley(pos, galley, color);
    response
}

/// Thin horizontal rule with generous breathing room.
fn divider(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 9.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, th::hairline()),
    );
}

/// Draw a drop highlight around `rect`.
fn drop_highlight(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(
        rect,
        th::corner(7),
        egui::Stroke::new(1.5_f32, th::accent()),
        egui::StrokeKind::Inside,
    );
}

/// Actions triggered by the left browser panel.
enum Act {
    Genre(Option<String>),
    Artist(Option<String>),
    Album(Option<String>),
    Library,
    Playlist(i64),
    Smart(i64),
    DropOnPlaylist(i64, Vec<i64>, Option<i64>),
    SaveView,
    NewEmptyPlaylist,
    NewPlaylistInFolder(i64),
    NewSmart,
    NewSmartInFolder(i64),
    EditSmart(i64),
    RenameSmart(i64),
    DeleteSmartAt(i64),
    ExportSmart(i64),
    MoveSmart(i64, Option<i64>),
    DeletePlaylist,
    DeletePlaylistAt(i64),
    NewFolder(Option<i64>),
    RenameFolder(i64),
    DeleteFolder(i64),
    RenamePlaylist(i64),
    MovePlaylist(i64, Option<i64>),
    ExportPlaylist(i64),
    ImportM3u,
    Podcasts,
    Radio,
}

/// `Parent / Child` label for folder menus.
fn folder_menu_path(folders: &[PlaylistFolder], folder_id: i64) -> String {
    let mut names = Vec::new();
    let mut current = Some(folder_id);
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        let Some(folder) = folders.iter().find(|folder| folder.id == id) else {
            break;
        };
        names.push(folder.name.clone());
        current = folder.parent_id;
    }
    names.reverse();
    names.join(" / ")
}

/// Render folders and playlists recursively. Returns the action the user
/// triggered, if any.
#[allow(clippy::too_many_arguments)]
fn playlist_tree(
    ui: &mut egui::Ui,
    folders: &[PlaylistFolder],
    playlists: &[Playlist],
    smart_playlists: &[SmartPlaylist],
    view: View,
    parent: Option<i64>,
    depth: usize,
    os_zones: &mut Vec<crate::app::OsDropZone>,
) -> Option<Act> {
    let indent = depth as f32 * 12.0;

    for folder in folders.iter().filter(|folder| folder.parent_id == parent) {
        let mut row_act: Option<Act> = None;
        let resp = nav_row(
            ui,
            false,
            indent,
            format!("{} {}", icons::FOLDER.as_str(), folder.name),
        );
        if resp.dnd_hover_payload::<DragPlaylist>().is_some() {
            drop_highlight(ui, resp.rect);
        }
        if let Some(drag) = resp.dnd_release_payload::<DragPlaylist>() {
            row_act = Some(if drag.smart {
                Act::MoveSmart(drag.id, Some(folder.id))
            } else {
                Act::MovePlaylist(drag.id, Some(folder.id))
            });
        }
        resp.context_menu(|ui| {
            if ui.button(tr("ui.new_playlist_here")).clicked() {
                row_act = Some(Act::NewPlaylistInFolder(folder.id));
                ui.close();
            }
            if ui.button("New smart playlist here…").clicked() {
                row_act = Some(Act::NewSmartInFolder(folder.id));
                ui.close();
            }
            if ui.button(tr("ui.new_subfolder")).clicked() {
                row_act = Some(Act::NewFolder(Some(folder.id)));
                ui.close();
            }
            if ui.button(tr("ui.rename_folder")).clicked() {
                row_act = Some(Act::RenameFolder(folder.id));
                ui.close();
            }
            if ui.button(tr("ui.delete_folder")).clicked() {
                row_act = Some(Act::DeleteFolder(folder.id));
                ui.close();
            }
        });
        if row_act.is_some() {
            return row_act;
        }
        if let Some(action) = playlist_tree(
            ui,
            folders,
            playlists,
            smart_playlists,
            view,
            Some(folder.id),
            depth + 1,
            os_zones,
        ) {
            return Some(action);
        }
    }

    for playlist in playlists.iter().filter(|p| p.folder_id == parent) {
        let mut row_act: Option<Act> = None;
        let selected = matches!(view, View::Playlist(id) if id == playlist.id);
        let label = format!(
            "{} {} ({})",
            icons::PLAYLIST.as_str(),
            playlist.name,
            playlist.count
        );
        let resp = nav_row(ui, selected, indent + 12.0, label);
        os_zones.push(crate::app::OsDropZone {
            playlist: Some(playlist.id),
            rect: resp.rect,
        });
        resp.dnd_set_drag_payload(DragPlaylist {
            id: playlist.id,
            smart: false,
        });

        if resp.dnd_hover_payload::<DragTracks>().is_some() || os_files_over(ui, resp.rect) {
            drop_highlight(ui, resp.rect);
        }
        if let Some(drag) = resp.dnd_release_payload::<DragTracks>() {
            row_act = Some(Act::DropOnPlaylist(
                playlist.id,
                drag.ids.clone(),
                drag.from_playlist,
            ));
        }

        resp.context_menu(|ui| {
            if ui.button(tr("ui.rename_2")).clicked() {
                row_act = Some(Act::RenamePlaylist(playlist.id));
                ui.close();
            }
            if ui.button(tr("ui.export_m3u")).clicked() {
                row_act = Some(Act::ExportPlaylist(playlist.id));
                ui.close();
            }
            ui.menu_button(tr("ui.move_to_folder"), |ui| {
                if playlist.folder_id.is_some() && ui.button(tr("ui.top_level")).clicked() {
                    row_act = Some(Act::MovePlaylist(playlist.id, None));
                    ui.close();
                }
                for folder in folders {
                    let label = folder_menu_path(folders, folder.id);
                    if ui.button(label).clicked() {
                        row_act = Some(Act::MovePlaylist(playlist.id, Some(folder.id)));
                        ui.close();
                    }
                }
            });
            if ui.button(tr("ui.delete_playlist")).clicked() {
                row_act = Some(Act::DeletePlaylistAt(playlist.id));
                ui.close();
            }
        });

        if resp.clicked() {
            row_act = Some(Act::Playlist(playlist.id));
        }
        if row_act.is_some() {
            return row_act;
        }
    }

    for smart in smart_playlists
        .iter()
        .filter(|playlist| playlist.folder_id == parent)
    {
        let mut row_act: Option<Act> = None;
        let selected = matches!(view, View::Smart(id) if id == smart.id);
        let label = format!("{} {}", icons::SPARKLE.as_str(), smart.name);
        let resp = nav_row(ui, selected, indent + 12.0, label);
        resp.dnd_set_drag_payload(DragPlaylist {
            id: smart.id,
            smart: true,
        });
        resp.context_menu(|ui| {
            if ui.button("Edit rules…").clicked() {
                row_act = Some(Act::EditSmart(smart.id));
                ui.close();
            }
            if ui.button(tr("ui.rename_2")).clicked() {
                row_act = Some(Act::RenameSmart(smart.id));
                ui.close();
            }
            if ui.button(tr("ui.export_m3u")).clicked() {
                row_act = Some(Act::ExportSmart(smart.id));
                ui.close();
            }
            ui.menu_button(tr("ui.move_to_folder"), |ui| {
                if smart.folder_id.is_some() && ui.button(tr("ui.top_level")).clicked() {
                    row_act = Some(Act::MoveSmart(smart.id, None));
                    ui.close();
                }
                for folder in folders {
                    let label = folder_menu_path(folders, folder.id);
                    if ui.button(label).clicked() {
                        row_act = Some(Act::MoveSmart(smart.id, Some(folder.id)));
                        ui.close();
                    }
                }
            });
            if ui.button(tr("ui.delete_playlist")).clicked() {
                row_act = Some(Act::DeleteSmartAt(smart.id));
                ui.close();
            }
        });
        if resp.clicked() {
            row_act = Some(Act::Smart(smart.id));
        }
        if row_act.is_some() {
            return row_act;
        }
    }

    None
}

/// Folder picker + background scan controls, shown in Settings.
fn scan_controls(app: &mut App, ui: &mut egui::Ui) {
    let scanning = app.is_scanning();
    ui.horizontal(|ui| {
        let path = ui.add_sized(
            [260.0, 24.0],
            egui::TextEdit::singleline(&mut app.scan_path)
                .id(egui::Id::new("scan_path"))
                .hint_text(tr("ui.folder_to_scan")),
        );
        let folder = icons::compact(ui, icons::FOLDER_OPEN, tr("ui.choose_a_music_folder"));
        if folder.clicked() {
            app.open_picker();
        }
        path.labelled_by(folder.id);
        if icons::button_enabled(
            ui,
            !scanning,
            icons::ARROWS_CLOCKWISE,
            if scanning {
                tr("library.indexing")
            } else {
                tr("library.scan")
            },
        )
        .on_hover_text("Index new and changed tracks in the background")
        .clicked()
        {
            app.scan_clicked();
        }
        if scanning {
            let spinner = ui.add(egui::Spinner::new().size(14.0));
            if let Some(label) = app.scan_progress_label() {
                spinner.on_hover_text(label);
            }
            if let Some(progress) = app.scan_progress() {
                let fraction = if progress.total > 0 {
                    (progress.found as f32 / progress.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let text = if progress.total == 0 {
                    "counting…".to_string()
                } else {
                    format!("{}/{}", progress.found, progress.total)
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(110.0)
                        .text(RichText::new(text).size(9.0)),
                );
            }
            if icons::compact(ui, icons::X, "Stop indexing").clicked() {
                app.cancel_scan();
            }
        }
    });
}

pub fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar")
        .frame(
            egui::Frame::new()
                .fill(th::bg_panel())
                .inner_margin(egui::Margin::symmetric(12, 7)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(logo) = &app.logo {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    ui.painter().image(
                        logo.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.label(th::semibold("MusicWasp", 18.0).color(th::text()));
                egui::Frame::new()
                    .fill(th::raised())
                    .corner_radius(th::corner(5))
                    .inner_margin(egui::Margin::symmetric(5, 1))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(9.0)
                                .color(th::text_dim()),
                        );
                    });
                ui.add_space(10.0);
                let mut view_label = app.view_title();
                if view_label.chars().count() > 26 {
                    view_label = view_label.chars().take(25).collect::<String>() + "…";
                }
                ui.label(th::semibold(view_label, 12.5).color(th::text_dim()));
                ui.add_space(8.0);

                let mut search_label_id = None;
                let search = egui::Frame::new()
                    .fill(th::raised())
                    .stroke(egui::Stroke::new(1.0, th::hairline()))
                    .corner_radius(th::pill())
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let icon = ui
                                .label(
                                    icons::text(icons::MAGNIFYING_GLASS, 13.0)
                                        .color(th::text_dim()),
                                )
                                .on_hover_text(
                                    "Advanced search: artist:\"radiohead\" year:1997\n\
                                 Fields: title, artist, album, albumartist, genre, format,\n\
                                 year, track, disc, duration. Use >, <, >=, <= or 1990..1999.",
                                );
                            icon.widget_info(|| {
                                WidgetInfo::labeled(WidgetType::Label, true, tr("ui.search_tracks"))
                            });
                            search_label_id = Some(icon.id);
                            ui.add_sized(
                                [168.0, 20.0],
                                egui::TextEdit::singleline(&mut app.search)
                                    .id(egui::Id::new("search_box"))
                                    .frame(egui::Frame::new())
                                    .margin(egui::Margin::ZERO)
                                    .hint_text(tr("ui.search_or_artist_radiohead_year_1997")),
                            )
                        })
                        .inner
                    })
                    .inner;
                let search = if let Some(id) = search_label_id {
                    search.labelled_by(id)
                } else {
                    search
                };
                if search.changed() {
                    app.mark_tracks_dirty();
                }
                if app.focus_search {
                    search.request_focus();
                    app.focus_search = false;
                }

                let mut save_search = false;
                let mut apply_saved: Option<String> = None;
                let mut edit_saved: Option<i64> = None;
                let mut delete_saved: Option<i64> = None;
                let saved = app.saved_searches.clone();
                let saved_menu = ui.menu_button(icons::text(icons::BOOKMARK_SIMPLE, 14.0), |ui| {
                    if ui.button(tr("ui.save_current_search")).clicked() {
                        save_search = true;
                        ui.close();
                    }
                    if !saved.is_empty() {
                        ui.separator();
                    }
                    for saved in &saved {
                        let response = ui
                            .button(format!("{} {}", icons::STAR.as_str(), saved.name))
                            .on_hover_text(&saved.query);
                        if response.clicked() {
                            apply_saved = Some(saved.query.clone());
                            ui.close();
                        }
                        response.context_menu(|ui| {
                            if ui.button(tr("ui.edit")).clicked() {
                                edit_saved = Some(saved.id);
                                ui.close();
                            }
                            if ui.button(tr("common.delete")).clicked() {
                                delete_saved = Some(saved.id);
                                ui.close();
                            }
                        });
                    }
                });
                let saved_menu = saved_menu.response;
                saved_menu.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, true, tr("ui.saved_searches"))
                });
                saved_menu.on_hover_text(tr("ui.saved_searches"));
                if save_search {
                    app.open_save_search_dialog();
                }
                if let Some(query) = apply_saved {
                    app.apply_search(&query);
                }
                if let Some(id) = edit_saved {
                    app.edit_saved_search(id);
                }
                if let Some(id) = delete_saved {
                    app.delete_saved_search(id);
                }

                let selected = app.selected_in_view().len();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let menu = ui.menu_button(icons::text(icons::LIST, 16.0), |ui| {
                        let settings = ui.add(
                            egui::Button::new(icons::with(icons::GEAR, tr("settings.title")))
                                .selected(app.show_settings),
                        );
                        if settings.clicked() {
                            app.show_settings = !app.show_settings;
                            ui.close();
                        }
                        let hygiene = ui.add(
                            egui::Button::new(icons::with(
                                icons::BROOM,
                                tr("menu.library_hygiene"),
                            ))
                            .selected(app.show_hygiene),
                        );
                        if hygiene.clicked() {
                            app.open_hygiene();
                            ui.close();
                        }
                        let organizer = ui.add(
                            egui::Button::new(icons::with(
                                icons::FOLDER,
                                "Organize files from tags…",
                            ))
                            .selected(app.show_organize),
                        );
                        if organizer.clicked() {
                            app.open_organize();
                            ui.close();
                        }
                        if app.tag_undo.is_some() {
                            let count = app.tag_undo.as_ref().map_or(0, Vec::len);
                            let label = format!("{} ({count})", tr("ui.undo_last_tag_write"));
                            let undo = ui.add(egui::Button::new(icons::with(
                                icons::ARROW_COUNTER_CLOCKWISE,
                                &label,
                            )));
                            if undo.clicked() {
                                app.undo_last_tag_write();
                                ui.close();
                            }
                        }
                        if let Some(count) = app.move_undo.as_ref().map(Vec::len) {
                            let undo = ui.add(egui::Button::new(icons::with(
                                icons::ARROW_COUNTER_CLOCKWISE,
                                &format!("Undo file moves ({count})"),
                            )));
                            if undo.clicked() {
                                app.undo_file_moves();
                                ui.close();
                            }
                        }
                        ui.separator();
                        let plain = ui.add(
                            egui::Button::new(icons::with(
                                icons::MONITOR_PLAY,
                                if app.plain_view {
                                    tr("ui.back_to_the_library")
                                } else {
                                    tr("ui.plain_now_playing_view")
                                },
                            ))
                            .selected(app.plain_view),
                        );
                        if plain.clicked() {
                            app.plain_view = !app.plain_view;
                            ui.close();
                        }
                        let mini = ui.add(
                            egui::Button::new(icons::with(icons::MONITOR_PLAY, "Mini player"))
                                .selected(app.settings.mini_player),
                        );
                        if mini.clicked() {
                            let enabled = !app.settings.mini_player;
                            app.set_mini_player(enabled, ui.ctx());
                            ui.close();
                        }
                        #[cfg(target_os = "linux")]
                        {
                            let tray = ui.add(
                                egui::Button::new(icons::with(icons::MONITOR_PLAY, "Tray icon"))
                                    .selected(app.settings.tray_enabled),
                            );
                            if tray.clicked() {
                                let enabled = !app.settings.tray_enabled;
                                app.set_tray_enabled(enabled, ui.ctx());
                                ui.close();
                            }
                        }
                        if ui
                            .add_enabled(
                                !app.plain_view,
                                egui::Button::new(icons::with(icons::QUEUE, tr("menu.play_queue")))
                                    .selected(app.settings.queue_open),
                            )
                            .clicked()
                        {
                            app.settings.queue_open = !app.settings.queue_open;
                            app.persist_settings();
                            ui.close();
                        }
                        if ui
                            .add(
                                egui::Button::new(icons::with(icons::QUOTES, "Lyrics"))
                                    .selected(app.settings.lyrics_open),
                            )
                            .clicked()
                        {
                            app.toggle_lyrics();
                            ui.close();
                        }
                        let columns = ui.add(
                            egui::Button::new(icons::with(
                                icons::COLUMNS,
                                tr("menu.track_columns"),
                            ))
                            .selected(app.show_columns),
                        );
                        if columns.clicked() {
                            app.show_columns = !app.show_columns;
                            ui.close();
                        }
                        let grid = ui.add(
                            egui::Button::new(icons::with(
                                icons::GRID_FOUR,
                                if app.album_view {
                                    tr("ui.back_to_the_track_list")
                                } else {
                                    tr("menu.album_grid")
                                },
                            ))
                            .selected(app.album_view),
                        );
                        if grid.clicked() {
                            app.toggle_album_view();
                            ui.close();
                        }
                        let visualizer = ui.add(
                            egui::Button::new(icons::with(icons::MONITOR_PLAY, "Visualizer"))
                                .selected(app.settings.visualizer.enabled),
                        );
                        if visualizer.clicked() {
                            let enabled = !app.settings.visualizer.enabled;
                            app.set_visualizer_enabled(enabled, ui.ctx());
                            ui.close();
                        }
                        if app.settings.visualizer.enabled
                            && ui
                                .button(icons::with(icons::MONITOR_PLAY, "Fullscreen visualizer"))
                                .clicked()
                        {
                            app.viz_fullscreen = true;
                            ui.close();
                        }
                        let podcasts = ui.add(
                            egui::Button::new(icons::with(icons::MICROPHONE, "Podcasts"))
                                .selected(matches!(app.view, View::Podcasts)),
                        );
                        if podcasts.clicked() {
                            app.view = View::Podcasts;
                            app.refresh_podcast_feeds();
                            ui.close();
                        }
                        let radio = ui.add(
                            egui::Button::new(icons::with(icons::BROADCAST, "Radio"))
                                .selected(matches!(app.view, View::Radio)),
                        );
                        if radio.clicked() {
                            app.view = View::Radio;
                            app.refresh_radio_stations();
                            ui.close();
                        }
                        let eq = ui.add(
                            egui::Button::new(icons::with(icons::SLIDERS, tr("menu.equalizer")))
                                .selected(app.show_equalizer),
                        );
                        if eq.clicked() {
                            app.show_equalizer = !app.show_equalizer;
                            ui.close();
                        }
                        ui.separator();
                        ui.menu_button(
                            icons::with(icons::SORT_ASCENDING, tr("menu.sort_by")),
                            |ui| {
                                for (key, label) in [
                                    (SortKey::Rating, tr("column.rating")),
                                    (SortKey::PlayCount, tr("sort.play_count")),
                                    (SortKey::LastPlayed, tr("sort.last_played")),
                                    (SortKey::Added, tr("sort.date_added")),
                                ] {
                                    if ui
                                        .add(egui::Button::new(label).selected(app.sort == key))
                                        .clicked()
                                    {
                                        apply_sort(app, key);
                                        ui.close();
                                    }
                                }
                            },
                        );
                        ui.menu_button(icons::with(icons::FADERS, tr("menu.play_history")), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(tr("filter.never_played"))
                                        .selected(app.filter_never_played),
                                )
                                .clicked()
                            {
                                app.filter_never_played = !app.filter_never_played;
                                app.mark_tracks_dirty();
                                ui.close();
                            }
                            if ui
                                .add(
                                    egui::Button::new(tr("filter.not_played_90"))
                                        .selected(app.filter_not_played_90),
                                )
                                .clicked()
                            {
                                app.filter_not_played_90 = !app.filter_not_played_90;
                                app.mark_tracks_dirty();
                                ui.close();
                            }
                            if app.filter_never_played || app.filter_not_played_90 {
                                ui.separator();
                                if ui.button(tr("filter.clear")).clicked() {
                                    app.filter_never_played = false;
                                    app.filter_not_played_90 = false;
                                    app.mark_tracks_dirty();
                                    ui.close();
                                }
                            }
                        });
                    });
                    let menu = menu.response;
                    menu.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Button, true, tr("menu.main"))
                    });
                    menu.on_hover_text(tr("menu.main"));
                    ui.add_space(4.0);
                    let (chip_text, chip_color) =
                        if !app.message.is_empty() && ui.available_width() > 180.0 {
                            (Some(app.message.clone()), th::accent())
                        } else if ui.available_width() < 210.0 {
                            // Not enough room: the table already shows what matters.
                            (None, th::text_dim())
                        } else if selected > 1 {
                            (Some(format!("{selected} selected")), th::accent())
                        } else {
                            app.lib
                                .stats()
                                .ok()
                                .map(|s| {
                                    (
                                        format!(
                                            "{} tracks · {} albums · {} artists · {}",
                                            s.tracks,
                                            s.albums,
                                            s.artists,
                                            fmt_size(s.total_bytes)
                                        ),
                                        th::text_dim(),
                                    )
                                })
                                .map_or((None, th::text_dim()), |(text, color)| (Some(text), color))
                        };
                    if let Some(text) = chip_text {
                        egui::Frame::new()
                            .fill(th::raised())
                            .corner_radius(th::pill())
                            .inner_margin(egui::Margin::symmetric(10, 3))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(text).size(10.5).color(chip_color),
                                    )
                                    .truncate(),
                                );
                            });
                    }
                });
            });
        });
}

pub fn browser(app: &mut App, ui: &mut egui::Ui) {
    let mut act: Option<Act> = None;
    let mut os_zones: Vec<crate::app::OsDropZone> = Vec::new();
    let mut folded = app.settings.folded;
    let mut folded_changed = false;

    egui::Panel::left("browser")
        .resizable(true)
        .default_size(218.0)
        .min_size(150.0)
        .frame(
            egui::Frame::new()
                .fill(th::bg_panel())
                .inner_margin(egui::Margin::symmetric(8, 8)),
        )
        .show(ui, |ui| {
            let full_h = ui.available_height();

            let sections = [
                (icons::FUNNEL_SIMPLE, tr("browser.genre"), 0.25),
                (icons::PERSON, tr("browser.artist"), 0.31),
                (icons::VINYL_RECORD, tr("browser.album"), 0.24),
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
                            tr("ui.expand_this_section")
                        } else {
                            tr("ui.collapse_this_section")
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
                            ui.spacing_mut().item_spacing.y = 1.0;
                            if nav_row(ui, selected.is_none(), 0.0, tr("ui.all")).clicked() {
                                act = Some(match i {
                                    0 => Act::Genre(None),
                                    1 => Act::Artist(None),
                                    _ => Act::Album(None),
                                });
                            }
                            for item in items {
                                let sel = selected == Some(item);
                                if nav_row(ui, sel, 0.0, item.as_str()).clicked() {
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
                divider(ui);
            }

            // Playlists
            let mut playlists_toggled = false;
            ui.horizontal(|ui| {
                playlists_toggled |= fold_button(
                    ui,
                    folded.playlists,
                    if folded.playlists {
                        tr("ui.expand_playlists")
                    } else {
                        tr("ui.collapse_playlists")
                    },
                );
                playlists_toggled |= section_label(ui, icons::QUEUE, tr("browser.playlists"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let menu = ui.menu_button(icons::text(icons::PLUS, 14.0), |ui| {
                        if ui.button("Save current view as playlist").clicked() {
                            act = Some(Act::SaveView);
                            ui.close();
                        }
                        if ui.button(tr("ui.new_empty_playlist")).clicked() {
                            act = Some(Act::NewEmptyPlaylist);
                            ui.close();
                        }
                        if ui.button(tr("ui.new_playlist_folder")).clicked() {
                            act = Some(Act::NewFolder(None));
                            ui.close();
                        }
                        if ui.button(tr("ui.import_m3u_playlist")).clicked() {
                            act = Some(Act::ImportM3u);
                            ui.close();
                        }
                    });
                    let response = menu.response;
                    response.widget_info(|| {
                        WidgetInfo::labeled(
                            WidgetType::Button,
                            true,
                            "New playlist, folder or M3U import",
                        )
                    });
                    response.on_hover_text("New playlist, folder or M3U import");
                    if icons::compact(ui, icons::SPARKLE, "New smart playlist").clicked() {
                        act = Some(Act::NewSmart);
                    }
                    if matches!(
                        app.view,
                        crate::app::View::Playlist(_) | crate::app::View::Smart(_)
                    ) && icons::compact(ui, icons::TRASH, tr("ui.delete_playlist")).clicked()
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
                    .id_salt(tr("browser.playlists"))
                    .auto_shrink([false, true])
                    .max_height(height_for(0.20).max(70.0))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let in_library = matches!(app.view, crate::app::View::Library);
                        let all = nav_row(
                            ui,
                            in_library,
                            0.0,
                            icons::with(icons::MUSIC_NOTES, tr("ui.all_music")),
                        );
                        os_zones.push(crate::app::OsDropZone {
                            playlist: None,
                            rect: all.rect,
                        });
                        if os_files_over(ui, all.rect) {
                            drop_highlight(ui, all.rect);
                        }
                        if all.clicked() {
                            act = Some(Act::Library);
                        }
                        if let Some(action) = playlist_tree(
                            ui,
                            &app.playlist_folders,
                            &app.playlists,
                            &app.smart_playlists,
                            app.view,
                            None,
                            0,
                            &mut os_zones,
                        ) {
                            act = Some(action);
                        }

                        if app.playlists.is_empty()
                            && app.smart_playlists.is_empty()
                            && app.playlist_folders.is_empty()
                        {
                            ui.label(
                                RichText::new(tr("ui.no_playlists_yet"))
                                    .size(10.0)
                                    .color(th::text_dim()),
                            );
                        } else if egui::DragAndDrop::has_any_payload(ui.ctx()) {
                            ui.label(
                                RichText::new(tr("ui.drop_tracks_here"))
                                    .size(9.0)
                                    .color(th::accent()),
                            );
                        }
                    });
            }
            divider(ui);
            if nav_row(
                ui,
                matches!(app.view, View::Podcasts),
                0.0,
                icons::with(icons::MICROPHONE, "Podcasts"),
            )
            .clicked()
            {
                act = Some(Act::Podcasts);
            }
            if nav_row(
                ui,
                matches!(app.view, View::Radio),
                0.0,
                icons::with(icons::BROADCAST, "Radio"),
            )
            .clicked()
            {
                act = Some(Act::Radio);
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
            app.open_playlist_dialog(false, None);
        }
        Some(Act::NewEmptyPlaylist) => {
            app.open_playlist_dialog(true, None);
        }
        Some(Act::NewPlaylistInFolder(folder)) => {
            app.open_playlist_dialog(true, Some(folder));
        }
        Some(Act::NewSmart) => app.open_smart_dialog(),
        Some(Act::NewSmartInFolder(folder)) => app.open_smart_dialog_in(Some(folder)),
        Some(Act::EditSmart(id)) => app.edit_smart_dialog(id),
        Some(Act::RenameSmart(id)) => app.start_rename_smart(id),
        Some(Act::DeleteSmartAt(id)) => app.delete_smart_by_id(id),
        Some(Act::ExportSmart(id)) => app.open_smart_m3u_export(id),
        Some(Act::MoveSmart(id, folder)) => app.move_smart_to_folder(id, folder),
        Some(Act::DeletePlaylist) => app.delete_current_playlist(),
        Some(Act::DeletePlaylistAt(id)) => app.delete_playlist_by_id(id),
        Some(Act::NewFolder(parent)) => app.open_new_folder(parent),
        Some(Act::RenameFolder(id)) => app.start_rename_folder(id),
        Some(Act::DeleteFolder(id)) => app.delete_playlist_folder(id),
        Some(Act::RenamePlaylist(id)) => app.start_rename_playlist(id),
        Some(Act::MovePlaylist(id, folder)) => app.move_playlist_to_folder(id, folder),
        Some(Act::ExportPlaylist(id)) => app.open_m3u_export(id),
        Some(Act::ImportM3u) => app.open_m3u_import(),
        Some(Act::Podcasts) => {
            app.view = View::Podcasts;
            app.refresh_podcast_feeds();
        }
        Some(Act::Radio) => {
            app.view = View::Radio;
            app.refresh_radio_stations();
        }
        None => {}
    }
}

/// Glyph sizes for the four transport buttons, per layout.
#[derive(Clone, Copy)]
struct TransportSize {
    side: f32,
    main: f32,
    stop: f32,
}

/// Previous, play/pause, next and stop, shared by the bottom bar and the
/// Now Playing screens.
fn transport_buttons(app: &mut App, ui: &mut egui::Ui, playing: bool, size: TransportSize) {
    if icons::transport(
        ui,
        icons::SKIP_BACK,
        size.side,
        false,
        tr("transport.previous"),
    )
    .clicked()
    {
        app.prev_track();
    }
    let icon = if playing { icons::PAUSE } else { icons::PLAY };
    if icons::transport(ui, icon, size.main, true, "Play / pause").clicked() {
        app.toggle_pause();
    }
    if icons::transport(
        ui,
        icons::SKIP_FORWARD,
        size.side,
        false,
        tr("transport.next"),
    )
    .clicked()
    {
        app.next_track();
    }
    if icons::transport(ui, icons::STOP, size.stop, false, tr("transport.stop")).clicked() {
        app.stop();
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

    let current_track = app.current_track(snap.current.as_deref());

    // Trust the tag reader's duration over the decoder's estimate.
    let known_duration = current_track
        .as_ref()
        .map(|t| t.duration_secs as f64)
        .filter(|d| *d > 0.0);
    let stream = app.stream.clone();
    let duration = stream
        .as_ref()
        .map(|stream| stream.duration_secs as f64)
        .filter(|duration| *duration > 0.0)
        .unwrap_or_else(|| known_duration.unwrap_or(snap.duration));

    egui::Panel::bottom("transport")
        .frame(
            egui::Frame::new()
                .fill(th::bg_panel())
                .inner_margin(egui::Margin::symmetric(14, 9)),
        )
        .show(ui, |ui| {
            ui.visuals_mut().selection.bg_fill = th::accent();
            ui.horizontal(|ui| {
                // Album art with a soft shadow so the bar reads as a raised surface.
                let art_size = 64.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(art_size, art_size), egui::Sense::hover());
                if !th::is_square() {
                    ui.painter().rect_filled(
                        rect.translate(egui::vec2(0.0, 3.0)).expand(1.0),
                        th::corner(10),
                        Color32::from_black_alpha(if th::skin().is_light() { 26 } else { 70 }),
                    );
                }
                if let Some(stream) = &stream {
                    ui.painter()
                        .rect_filled(rect, th::corner(8), th::bg_row_alt());
                    let icon = match stream.kind {
                        StreamKind::Radio => icons::BROADCAST,
                        StreamKind::Podcast => icons::MICROPHONE,
                    };
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon.as_str(),
                        egui::FontId::proportional(30.0),
                        th::accent(),
                    );
                } else {
                    match &app.art {
                        Some((_, tex)) => {
                            egui::Image::new((tex.id(), egui::vec2(art_size, art_size)))
                                .corner_radius(th::corner(8))
                                .paint_at(ui, rect);
                        }
                        None => {
                            ui.painter()
                                .rect_filled(rect, th::corner(8), th::bg_row_alt());
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icons::MUSIC_NOTES.as_str(),
                                egui::FontId::proportional(28.0),
                                th::text_dim(),
                            );
                        }
                    }
                }
                if !th::is_square() {
                    ui.painter().rect_stroke(
                        rect,
                        th::corner(8),
                        egui::Stroke::new(1.0, th::hairline()),
                        egui::StrokeKind::Inside,
                    );
                }

                ui.add_space(11.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title = stream
                        .as_ref()
                        .map(|stream| stream.title.clone())
                        .or_else(|| current_track.as_ref().map(|track| track.title.clone()))
                        .unwrap_or_else(|| tr("transport.nothing_playing").to_string());
                    let sub = stream
                        .as_ref()
                        .map(|stream| stream.subtitle.clone())
                        .or_else(|| current_track.as_ref().map(artist_album_line))
                        .unwrap_or_else(|| tr("transport.hint").to_string());
                    ui.add(
                        egui::Label::new(th::semibold(title, 14.5).color(th::text())).truncate(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(sub).size(11.0).color(th::text_dim()))
                            .truncate(),
                    );

                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let playing = snap.status == musicwasp_core::PlayerStatus::Playing;
                        transport_buttons(
                            app,
                            ui,
                            playing,
                            TransportSize {
                                side: 15.0,
                                main: 19.0,
                                stop: 12.0,
                            },
                        );
                        ui.add_space(7.0);

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
                        if icons::toggle(ui, app.shuffle, icons::SHUFFLE, tr("transport.shuffle"))
                            .clicked()
                        {
                            app.shuffle = !app.shuffle;
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    ui.spacing_mut().slider_width = 100.0;
                    let volume =
                        ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                    volume.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.volume"))
                    });
                    ui.label(icons::text(icons::SPEAKER_HIGH, 14.0).color(th::text_dim()))
                        .on_hover_text(tr("transport.volume"));
                    if icons::toggle(ui, app.settings.lyrics_open, icons::QUOTES, "Lyrics")
                        .clicked()
                    {
                        app.toggle_lyrics();
                    }
                });
            });

            ui.add_space(5.0);

            // Seek row: live streams have no seek bar.
            if let Some(stream) = &stream {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [42.0, 14.0],
                        egui::Label::new(
                            RichText::new(fmt_time(snap.position))
                                .monospace()
                                .size(10.5)
                                .color(th::text_dim()),
                        ),
                    );
                    match stream.kind {
                        StreamKind::Radio => {
                            ui.label(RichText::new("● LIVE").size(10.5).color(th::accent()));
                        }
                        StreamKind::Podcast => {
                            ui.label(
                                RichText::new(format!(
                                    "STREAMING  ·  {}",
                                    if duration > 0.0 {
                                        fmt_time(duration)
                                    } else {
                                        "unknown length".to_string()
                                    }
                                ))
                                .size(10.5)
                                .color(th::text_dim()),
                            );
                        }
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [42.0, 14.0],
                        egui::Label::new(
                            RichText::new(fmt_time(app.seek_drag.unwrap_or(snap.position)))
                                .monospace()
                                .size(10.5)
                                .color(th::text_dim()),
                        ),
                    );
                    let mut pos = app.seek_drag.unwrap_or(snap.position);
                    let max = duration.max(1.0);
                    ui.spacing_mut().slider_width = (ui.available_width() - 100.0).max(100.0);
                    let resp = ui.add(
                        egui::Slider::new(&mut pos, 0.0..=max)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    resp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.seek"))
                    });
                    if resp.dragged() {
                        app.seek_drag = Some(pos);
                    }
                    if resp.drag_stopped() {
                        if let Some(p) = &app.player {
                            p.seek(pos);
                        }
                        app.seek_drag = None;
                    }
                    ui.add_sized(
                        [42.0, 14.0],
                        egui::Label::new(
                            RichText::new(fmt_time(duration))
                                .monospace()
                                .size(10.5)
                                .color(th::text_dim()),
                        ),
                    );
                });
            }

            if let Some(err) = &app.audio_error {
                ui.label(
                    RichText::new(format!("audio unavailable: {err}"))
                        .size(10.0)
                        .color(Color32::from_rgb(220, 110, 110)),
                );
            }
        });

    // Volume changed by the slider above is pushed to the engine next frame.
    if let Some(p) = &app.player {
        if (app.volume - app.volume_sent).abs() > f32::EPSILON {
            p.set_volume(app.volume);
            app.volume_sent = app.volume;
        }
    }
}

/// Right-hand lyrics panel. Synchronized lines highlight and auto-scroll with
/// playback; clicking a line seeks to it.
pub fn lyrics_panel(app: &mut App, ui: &mut egui::Ui) {
    if !app.settings.lyrics_open {
        return;
    }

    let snapshot =
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
    let active = app
        .lyrics
        .as_ref()
        .and_then(|lyrics| lyrics.line_index(snapshot.position));
    let current_track = app.current_track(snapshot.current.as_deref());
    let stream = app.stream.clone();

    let mut close = false;
    let mut reload = false;
    let mut seek_to = None;

    egui::Panel::right("lyrics")
        .resizable(true)
        .default_size(320.0)
        .min_size(230.0)
        .max_size(560.0)
        .frame(
            egui::Frame::new()
                .fill(th::bg_panel())
                .inner_margin(egui::Margin::symmetric(12, 9)),
        )
        .show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    th::semibold(format!("{} LYRICS", icons::QUOTES.as_str()), 10.0)
                        .color(th::text_dim()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::compact(ui, icons::X, "Hide lyrics").clicked() {
                        close = true;
                    }
                    if icons::compact(ui, icons::ARROWS_CLOCKWISE, "Reload lyrics").clicked() {
                        reload = true;
                    }
                    if stream.is_none()
                        && app.lyrics.is_some()
                        && icons::toggle(
                            ui,
                            app.lyrics_autoscroll,
                            icons::ARROW_DOWN,
                            "Auto-scroll",
                        )
                        .clicked()
                    {
                        app.lyrics_autoscroll = !app.lyrics_autoscroll;
                    }
                });
            });
            divider(ui);

            if stream.is_some() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(icons::text(icons::QUOTES, 32.0).color(th::text_dim()));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Lyrics are available for local tracks.")
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                });
                return;
            }

            let Some(lyrics) = app.lyrics.as_ref() else {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(icons::text(icons::QUOTES, 32.0).color(th::text_dim()));
                    ui.add_space(8.0);
                    ui.label(th::semibold("No lyrics found", 13.0).color(th::text()));
                    ui.add_space(4.0);
                    let hint = current_track
                        .as_ref()
                        .map(|track| {
                            let path = std::path::Path::new(&track.path).with_extension("lrc");
                            format!("Add {} next to the audio file.", path.display())
                        })
                        .unwrap_or_else(|| "Start a track to show its .lrc lyrics.".to_string());
                    ui.label(RichText::new(hint).size(10.0).color(th::text_dim()));
                });
                return;
            };

            if lyrics.lines.is_empty() {
                return;
            }

            let synced = lyrics.synced;
            egui::ScrollArea::vertical()
                .id_salt("lyrics_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(60.0);
                    for (index, line) in lyrics.lines.iter().enumerate() {
                        let is_active = active == Some(index);
                        let text = if is_active {
                            th::semibold(&line.text, 16.0).color(th::accent())
                        } else {
                            RichText::new(&line.text).size(13.0).color(if !synced {
                                th::text()
                            } else if active.is_none_or(|active| index < active) {
                                th::text_dim()
                            } else {
                                th::text()
                            })
                        };
                        let mut label = egui::Label::new(text).wrap();
                        if synced {
                            label = label.sense(egui::Sense::click());
                        }
                        let inner = ui.vertical_centered(|ui| ui.add(label)).inner;
                        let rect = inner.rect;
                        if synced {
                            let response = inner
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(format!(
                                    "Seek to {}",
                                    fmt_time(line.time_ms as f64 / 1000.0)
                                ));
                            if response.clicked() {
                                seek_to = Some(line.time_ms);
                            }
                        }
                        if is_active && app.lyrics_autoscroll && app.lyrics_active != active {
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                        ui.add_space(9.0);
                    }
                    ui.add_space(80.0);
                });
        });

    if close {
        app.settings.lyrics_open = false;
        app.lyrics_active = None;
        app.persist_settings();
    }
    if reload {
        app.reload_lyrics();
    } else {
        app.lyrics_active = active;
    }
    if let Some(time_ms) = seek_to {
        if let Some(player) = &app.player {
            player.seek(time_ms as f64 / 1000.0);
        }
        app.seek_drag = None;
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
    let current_track = app.current_track(snap.current.as_deref());
    let known_duration = current_track
        .as_ref()
        .map(|t| t.duration_secs as f64)
        .filter(|d| *d > 0.0);
    let duration = known_duration.unwrap_or(snap.duration);
    let upcoming = app.upcoming_track();
    let playing = snap.status == musicwasp_core::PlayerStatus::Playing;

    egui::CentralPanel::default().show(ui, |ui| {
        let avail = ui.available_size();
        if app.settings.visualizer.enabled {
            let rect = ui.max_rect();
            if let Some(texture) = app.render_visualizer(ui.ctx(), rect.size()) {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::ZERO,
                    Color32::from_black_alpha(if th::skin().is_light() { 90 } else { 150 }),
                );
            }
        }
        if let Some(stream) = app.stream.clone() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                let art = (avail.y * 0.34).clamp(150.0, 280.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(art, art), egui::Sense::hover());
                let icon = match stream.kind {
                    StreamKind::Radio => icons::BROADCAST,
                    StreamKind::Podcast => icons::MICROPHONE,
                };
                ui.painter()
                    .rect_filled(rect, th::corner(14), th::bg_row_alt());
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon.as_str(),
                    egui::FontId::proportional(art * 0.34),
                    th::accent(),
                );
                ui.add_space(20.0);
                ui.add(
                    egui::Label::new(th::semibold(&stream.title, 25.0).color(th::text()))
                        .truncate(),
                );
                ui.label(
                    RichText::new(&stream.subtitle)
                        .size(14.0)
                        .color(th::text_dim()),
                );
                ui.label(
                    RichText::new(match stream.kind {
                        StreamKind::Radio => "Internet radio",
                        StreamKind::Podcast => "Podcast",
                    })
                    .size(10.5)
                    .color(th::text_dim()),
                );
                ui.add_space(22.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(230.0, 46.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let playing = snap.status == musicwasp_core::PlayerStatus::Playing;
                        transport_buttons(
                            app,
                            ui,
                            playing,
                            TransportSize {
                                side: 18.0,
                                main: 28.0,
                                stop: 16.0,
                            },
                        );
                    },
                );
                ui.add_space(14.0);
                let position = fmt_time(snap.position);
                if stream.kind == StreamKind::Radio {
                    ui.label(
                        RichText::new(format!("{position}  ·  ● LIVE"))
                            .size(11.5)
                            .color(th::accent()),
                    );
                } else if stream.duration_secs > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{position} / {}",
                            fmt_time(stream.duration_secs as f64)
                        ))
                        .size(11.5)
                        .color(th::text_dim()),
                    );
                } else {
                    ui.label(RichText::new(position).size(11.5).color(th::text_dim()));
                }
            });
            return;
        }
        let art = (avail.y * 0.40).clamp(140.0, 330.0).min(avail.x * 0.42);

        ui.vertical_centered(|ui| {
            ui.visuals_mut().selection.bg_fill = th::accent();
            ui.add_space(10.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(art, art), egui::Sense::hover());
            if !th::is_square() {
                ui.painter().rect_filled(
                    rect.translate(egui::vec2(0.0, 6.0)).expand(2.0),
                    th::corner(14),
                    Color32::from_black_alpha(if th::skin().is_light() { 24 } else { 80 }),
                );
            }
            match &app.art {
                Some((_, tex)) => {
                    egui::Image::new((tex.id(), egui::vec2(art, art)))
                        .corner_radius(th::corner(12))
                        .paint_at(ui, rect);
                }
                None => {
                    ui.painter()
                        .rect_filled(rect, th::corner(12), th::bg_row_alt());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icons::MUSIC_NOTES.as_str(),
                        egui::FontId::proportional((art * 0.30).clamp(32.0, 96.0)),
                        th::text_dim(),
                    );
                }
            }
            if !th::is_square() {
                ui.painter().rect_stroke(
                    rect,
                    th::corner(12),
                    egui::Stroke::new(1.0_f32, th::hairline()),
                    egui::StrokeKind::Inside,
                );
            }

            ui.add_space(20.0);
            let title = current_track
                .as_ref()
                .map(|t| t.title.clone())
                .unwrap_or_else(|| tr("transport.nothing_playing").to_string());
            ui.label(th::semibold(title, 27.0).color(th::text()));

            ui.add_space(2.0);
            let subtitle = current_track
                .as_ref()
                .map(artist_album_line)
                .unwrap_or_else(|| "Pick a track in the library to start".to_string());
            ui.label(RichText::new(subtitle).size(14.0).color(th::text_dim()));
            if let Some(track) = &current_track {
                let mut rating = None;
                ui.allocate_ui_with_layout(
                    egui::vec2(5.0 * 20.0, 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        rating = rating_stars(ui, track.rating, true, 18.0);
                    },
                );
                if let Some(rating) = rating {
                    app.set_track_rating(track.id, rating);
                }
            }

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
                    resp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.seek"))
                    });
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
            ui.allocate_ui(egui::vec2(330.0, 34.0), |ui| {
                ui.horizontal(|ui| {
                    transport_buttons(
                        app,
                        ui,
                        playing,
                        TransportSize {
                            side: 22.0,
                            main: 30.0,
                            stop: 18.0,
                        },
                    );
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
                    if icons::toggle(ui, app.shuffle, icons::SHUFFLE, tr("transport.shuffle"))
                        .clicked()
                    {
                        app.shuffle = !app.shuffle;
                    }
                    if icons::toggle(ui, app.settings.lyrics_open, icons::QUOTES, "Lyrics")
                        .clicked()
                    {
                        app.toggle_lyrics();
                    }
                });
            });

            ui.add_space(12.0);
            ui.allocate_ui(egui::vec2(160.0, 24.0), |ui| {
                ui.horizontal(|ui| {
                    ui.label(icons::text(icons::SPEAKER_HIGH, 14.0).color(th::text_dim()));
                    ui.spacing_mut().slider_width = 120.0;
                    let volume =
                        ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                    volume.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.volume"))
                    });
                });
            });

            ui.add_space(26.0);
            match &upcoming {
                Some(track) => {
                    ui.label(
                        th::semibold(
                            format!("{} {}", icons::QUEUE.as_str(), tr("queue.up_next_upper")),
                            10.0,
                        )
                        .color(th::text_dim()),
                    );
                    ui.add_space(4.0);
                    ui.label(th::semibold(&track.title, 15.0).color(th::text()));
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
            .frame(
                egui::Frame::new()
                    .fill(th::bg_panel())
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        th::semibold(
                            format!("{} {}", icons::QUEUE.as_str(), tr("queue.title")),
                            10.0,
                        )
                        .color(th::text_dim()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::compact(ui, icons::X, tr("ui.hide_the_play_queue")).clicked() {
                            act = Some(Act::Close);
                        }
                        if icons::compact_enabled(
                            ui,
                            can_clear,
                            icons::TRASH,
                            tr("ui.clear_everything_after_the_current_track"),
                        )
                        .clicked()
                        {
                            act = Some(Act::Clear);
                        }
                    });
                });
                divider(ui);

                if let Some(track) = &current_track {
                    ui.add_space(4.0);
                    ui.label(th::semibold("NOW PLAYING", 9.5).color(th::accent()));
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(th::semibold(&track.title, 13.0).color(th::text()))
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
                    ui.add_space(4.0);
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(th::semibold(tr("queue.up_next_upper"), 9.5).color(th::text_dim()));
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
                        RichText::new(format!("{} {}", icons::CHECK.as_str(), tr("queue.empty")))
                            .size(11.0)
                            .color(th::text_dim()),
                    );
                    ui.label(
                        RichText::new(tr(
                            "ui.right_click_a_track_and_choose_play_next_or_add_to_queue",
                        ))
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
                                    egui::vec2(width, 24.0),
                                    egui::Sense::click_and_drag(),
                                );

                                if resp.hovered() {
                                    ui.painter()
                                        .rect_filled(rect, th::corner(6), th::hover_fill());
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

                                resp.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        true,
                                        format!(
                                            "Queue entry {}: {} by {}",
                                            display_row, track.title, track.artist
                                        ),
                                    )
                                });
                                let resp = resp.on_hover_text(format!(
                                    "Play now\n{} - {}\nDrag to reorder",
                                    track.title,
                                    fmt_time(track.duration_secs as f64)
                                ));
                                resp.context_menu(|ui| {
                                    if ui
                                        .button(icons::with(icons::PLAY, tr("ui.play_now")))
                                        .clicked()
                                    {
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

/// Album grid with cover tiles. Right-click a tile for the cover tools.
/// Podcast subscriptions and episodes.
pub fn podcasts_view(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .button(icons::with(icons::ARROW_LEFT, "Library"))
                .clicked()
            {
                app.view = View::Library;
                app.mark_browser_dirty();
            }
            ui.label(th::semibold("Podcasts", 18.0).color(th::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(icons::with(icons::PLUS, "Add feed")).clicked() {
                    app.show_podcast_dialog = true;
                }
                if app.selected_feed.is_some()
                    && ui
                        .add_enabled(!app.podcast_loading, egui::Button::new("Refresh"))
                        .clicked()
                {
                    app.refresh_selected_podcast();
                }
            });
        });
        if let Some(status) = &app.podcast_status {
            ui.label(
                RichText::new(status.as_str())
                    .size(10.5)
                    .color(th::text_dim()),
            );
        }
        if app.podcast_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Fetching feed…").size(11.0));
            });
        }
        ui.separator();

        if app.selected_feed.is_none() {
            if app.podcast_feeds.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("No podcast subscriptions yet. Add a feed URL.")
                            .size(12.0)
                            .color(th::text_dim()),
                    );
                });
                return;
            }
            enum FeedAction {
                Select(i64),
                Delete(i64),
            }
            let mut feed_action = None;
            egui::ScrollArea::vertical()
                .id_salt("podcast_feeds")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for index in 0..app.podcast_feeds.len() {
                        let feed = &app.podcast_feeds[index];
                        ui.horizontal(|ui| {
                            let label = format!(
                                "{}   ·   {} episode(s), {} unplayed",
                                feed.title, feed.episode_count, feed.unplayed_count
                            );
                            let width = (ui.available_width() - 36.0).max(80.0);
                            let response = ui.add_sized(
                                [width, 28.0],
                                egui::Label::new(RichText::new(label).size(12.5).color(th::text()))
                                    .sense(egui::Sense::click())
                                    .truncate(),
                            );
                            if response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                feed_action = Some(FeedAction::Select(feed.id));
                            }
                            if icons::compact(ui, icons::TRASH, "Delete subscription").clicked() {
                                feed_action = Some(FeedAction::Delete(feed.id));
                            }
                        });
                        ui.label(
                            RichText::new(feed.url.as_str())
                                .size(9.5)
                                .color(th::text_dim()),
                        );
                        ui.add_space(3.0);
                    }
                });
            if let Some(action) = feed_action {
                match action {
                    FeedAction::Select(id) => app.select_podcast_feed(id),
                    FeedAction::Delete(id) => app.delete_podcast_feed(id),
                }
            }
        } else {
            let feed_id = app.selected_feed.unwrap_or_default();
            let title = app
                .podcast_feeds
                .iter()
                .find(|feed| feed.id == feed_id)
                .map(|feed| feed.title.clone())
                .unwrap_or_else(|| "Feed".to_string());
            ui.horizontal(|ui| {
                if ui.button("All feeds").clicked() {
                    app.selected_feed = None;
                    app.podcast_episodes.clear();
                }
                ui.label(th::semibold(title, 15.0).color(th::text()));
                ui.label(
                    RichText::new(format!("{} episode(s)", app.podcast_episodes.len()))
                        .size(10.5)
                        .color(th::text_dim()),
                );
            });
            ui.add_space(4.0);
            if app.podcast_episodes.is_empty() {
                ui.label(
                    RichText::new("No episodes yet. Refresh the feed.")
                        .size(11.0)
                        .color(th::text_dim()),
                );
                return;
            }
            let mut play_episode = None;
            egui::ScrollArea::vertical()
                .id_salt("podcast_episodes")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for index in 0..app.podcast_episodes.len() {
                        let episode = &app.podcast_episodes[index];
                        ui.horizontal(|ui| {
                            let playing = app
                                .stream
                                .as_ref()
                                .is_some_and(|stream| stream.episode_id == Some(episode.id));
                            if icons::compact(
                                ui,
                                if playing { icons::PAUSE } else { icons::PLAY },
                                "Play episode",
                            )
                            .clicked()
                            {
                                play_episode = Some(index);
                            }
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        th::semibold(&episode.title, 12.5).color(th::text()),
                                    )
                                    .truncate(),
                                );
                                let date = if episode.published.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}  ·  ", episode.published)
                                };
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        date,
                                        if episode.description.is_empty() {
                                            String::new()
                                        } else {
                                            episode.description.chars().take(110).collect()
                                        }
                                    ))
                                    .size(10.0)
                                    .color(th::text_dim()),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if episode.played {
                                        ui.label(
                                            icons::text(icons::CHECK, 12.0).color(th::accent()),
                                        );
                                    } else if episode.position_secs > 0 {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} / {}",
                                                fmt_time(episode.position_secs as f64),
                                                fmt_time(episode.duration_secs as f64)
                                            ))
                                            .size(10.0)
                                            .color(th::text_dim()),
                                        );
                                    } else if episode.duration_secs > 0 {
                                        ui.label(
                                            RichText::new(fmt_time(episode.duration_secs as f64))
                                                .size(10.0)
                                                .color(th::text_dim()),
                                        );
                                    }
                                },
                            );
                        });
                        ui.separator();
                    }
                });
            if let Some(index) = play_episode {
                if let Some(episode) = app.podcast_episodes.get(index).cloned() {
                    app.play_podcast_episode(&episode);
                }
            }
        }
    });
    podcast_dialog(app, &ctx);
}

fn podcast_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.show_podcast_dialog {
        return;
    }
    let mut open = true;
    let mut url = app.podcast_url.clone();
    let mut add = false;
    egui::Window::new("Add podcast feed")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("RSS or Atom feed URL").size(10.5));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut url).hint_text("https://example.com/feed.xml"),
            );
            ui.label(
                RichText::new("The feed's episodes are stored in the library database.")
                    .size(9.5)
                    .color(th::text_dim()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Add feed").clicked() {
                    add = true;
                }
                if ui.button("Cancel").clicked() {
                    app.show_podcast_dialog = false;
                }
            });
        });
    app.podcast_url = url;
    if add {
        app.add_podcast_feed(&app.podcast_url.clone());
    }
    if !open {
        app.show_podcast_dialog = false;
    }
}

/// Internet radio station list.
pub fn radio_view(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .button(icons::with(icons::ARROW_LEFT, "Library"))
                .clicked()
            {
                app.view = View::Library;
                app.mark_browser_dirty();
            }
            ui.label(th::semibold("Radio", 18.0).color(th::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(icons::with(icons::PLUS, "Add station")).clicked() {
                    app.show_radio_dialog = true;
                }
            });
        });
        if let Some(status) = &app.radio_status {
            ui.label(
                RichText::new(status.as_str())
                    .size(10.5)
                    .color(th::text_dim()),
            );
        }
        ui.separator();
        if app.radio_stations.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No radio stations yet. Add an MP3 or AAC stream URL.")
                        .size(12.0)
                        .color(th::text_dim()),
                );
            });
            return;
        }
        enum StationAction {
            Play(usize),
            Delete(i64),
        }
        let mut station_action = None;
        egui::ScrollArea::vertical()
            .id_salt("radio_stations")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for index in 0..app.radio_stations.len() {
                    let station = &app.radio_stations[index];
                    ui.horizontal(|ui| {
                        let playing = app.stream.as_ref().is_some_and(|stream| {
                            stream.kind == StreamKind::Radio && stream.url == station.url
                        });
                        if icons::compact(
                            ui,
                            if playing { icons::PAUSE } else { icons::PLAY },
                            "Play station",
                        )
                        .clicked()
                        {
                            station_action = Some(StationAction::Play(index));
                        }
                        ui.vertical(|ui| {
                            ui.label(th::semibold(&station.name, 13.0).color(th::text()));
                            ui.label(
                                RichText::new(station.genre.as_str())
                                    .size(10.0)
                                    .color(th::text_dim()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::compact(ui, icons::TRASH, "Delete station").clicked() {
                                station_action = Some(StationAction::Delete(station.id));
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(station.url.as_str())
                                        .size(9.5)
                                        .color(th::text_dim()),
                                )
                                .truncate(),
                            );
                        });
                    });
                    ui.separator();
                }
            });
        match station_action {
            Some(StationAction::Play(index)) => {
                if let Some(station) = app.radio_stations.get(index).cloned() {
                    app.play_radio_station(&station);
                }
            }
            Some(StationAction::Delete(id)) => app.delete_radio_station(id),
            None => {}
        }
    });
    radio_dialog(app, &ctx);
}

fn radio_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.show_radio_dialog {
        return;
    }
    let mut open = true;
    let mut name = app.radio_name.clone();
    let mut url = app.radio_url.clone();
    let mut genre = app.radio_genre.clone();
    let mut add = false;
    egui::Window::new("Add radio station")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Name").size(10.5));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut name).hint_text("Station name"),
            );
            ui.label(RichText::new("Stream URL").size(10.5));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut url).hint_text("https://example.com/stream.mp3"),
            );
            ui.label(RichText::new("Genre (optional)").size(10.5));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut genre).hint_text("Jazz"),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Add station").clicked() {
                    add = true;
                }
                if ui.button("Cancel").clicked() {
                    app.show_radio_dialog = false;
                }
            });
        });
    app.radio_name = name;
    app.radio_url = url;
    app.radio_genre = genre;
    if add {
        let (name, url, genre) = (
            app.radio_name.clone(),
            app.radio_url.clone(),
            app.radio_genre.clone(),
        );
        app.add_radio_station(&name, &url, &genre);
    }
    if !open {
        app.show_radio_dialog = false;
    }
}

pub fn visualizer_panel(app: &mut App, ui: &mut egui::Ui) {
    if !app.settings.visualizer.enabled {
        return;
    }
    egui::Panel::bottom("visualizer")
        .resizable(true)
        .default_size(230.0)
        .min_size(140.0)
        .max_size(560.0)
        .frame(
            egui::Frame::new()
                .fill(th::bg_panel())
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(th::semibold("Visualizer", 10.5).color(th::text_dim()));
                if let Some(visualizer) = &app.visualizer {
                    ui.label(
                        RichText::new(visualizer.current_name())
                            .size(11.0)
                            .color(th::text()),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            visualizer.current_index() + 1,
                            visualizer.presets().len()
                        ))
                        .size(10.0)
                        .color(th::text_dim()),
                    );
                }
                if let Some(error) = &app.viz_error {
                    ui.label(
                        RichText::new(error.as_str())
                            .size(10.0)
                            .color(Color32::from_rgb(220, 110, 110)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::compact(ui, icons::X, "Hide the visualizer").clicked() {
                        app.set_visualizer_enabled(false, ui.ctx());
                    }
                    if ui.button("Fullscreen").clicked() {
                        app.viz_fullscreen = true;
                        ui.ctx().request_repaint();
                    }
                    if icons::compact(ui, icons::SKIP_FORWARD, "Next preset").clicked() {
                        app.next_visualizer_preset();
                    }
                    if icons::compact(ui, icons::SKIP_BACK, "Previous preset").clicked() {
                        app.previous_visualizer_preset();
                    }
                    let shuffle =
                        app.settings.visualizer.mode == crate::settings::VisualizerMode::Shuffle;
                    if icons::toggle(ui, shuffle, icons::SHUFFLE, "Shuffle presets").clicked() {
                        let mode = if shuffle {
                            crate::settings::VisualizerMode::Selected
                        } else {
                            crate::settings::VisualizerMode::Shuffle
                        };
                        app.set_visualizer_mode(mode);
                    }
                });
            });
            ui.add_space(4.0);
            let size = ui.available_size();
            if let Some(texture) = app.render_visualizer(ui.ctx(), size) {
                ui.add(egui::Image::new((texture, size)).corner_radius(th::corner(8)));
            } else {
                ui.centered_and_justified(|ui| {
                    let text = app.viz_error.as_deref().unwrap_or("Visualizer unavailable");
                    ui.label(RichText::new(text).size(11.0).color(th::text_dim()));
                });
            }
        });
}

/// Full-window visualizer with a floating control strip.
pub fn visualizer_fullscreen(app: &mut App, ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(Color32::BLACK))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            if let Some(texture) = app.render_visualizer(ui.ctx(), rect.size()) {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    app.viz_error.as_deref().unwrap_or("Visualizer unavailable"),
                    egui::FontId::proportional(16.0),
                    Color32::LIGHT_GRAY,
                );
            }

            let bar_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(16.0, 14.0),
                egui::vec2((rect.width() - 32.0).max(220.0), 40.0),
            );
            let mut bar = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            egui::Frame::new()
                .fill(Color32::from_black_alpha(145))
                .corner_radius(th::pill())
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(&mut bar, |ui| {
                    let name = app
                        .visualizer
                        .as_ref()
                        .map(|visualizer| visualizer.current_name().to_string())
                        .unwrap_or_default();
                    ui.label(th::semibold(name, 12.0).color(Color32::WHITE));
                    ui.add_space(8.0);
                    if ui.button("Previous").clicked() {
                        app.previous_visualizer_preset();
                    }
                    if ui.button("Next").clicked() {
                        app.next_visualizer_preset();
                    }
                    if ui.button("Exit fullscreen").clicked() {
                        app.viz_fullscreen = false;
                    }
                });
        });
}

pub fn album_grid(app: &mut App, ui: &mut egui::Ui) {
    // No CentralPanel wraps this view, so paint the surface explicitly. The
    // fill stays translucent for wallpaper/gradient skins.
    let background = ui.available_rect_before_wrap();
    ui.painter().rect_filled(
        background,
        egui::CornerRadius::ZERO,
        ui.visuals().panel_fill,
    );

    if app.album_groups.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(tr("ui.no_albums_yet_scan_a_music_folder_first"))
                    .size(13.0)
                    .color(th::text_dim()),
            );
        });
        return;
    }

    enum Act {
        Open(String),
        Fetch(String, String),
        Replace(String, String, PathBuf),
        Sidecar(String, String),
    }
    let mut act: Option<Act> = None;
    let ctx = ui.ctx().clone();
    // Borrow the groups instead of cloning every album each frame, and take the
    // cover cache out so disjoint fields can be borrowed together.
    let mut covers = std::mem::take(&mut app.album_covers);
    let groups = &app.album_groups;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} album(s)", groups.len()))
                .size(10.5)
                .color(th::text_dim()),
        );
        ui.label(
            RichText::new("Click to open · right-click for cover tools")
                .size(10.0)
                .color(th::text_dim()),
        );
    });

    let gap = 10.0;
    let tile_width = 184.0;
    let tile_height = 238.0;
    let columns = (((ui.available_width() + gap) / (tile_width + gap)).floor() as usize).max(1);
    let rows = groups.len().div_ceil(columns);

    egui::ScrollArea::vertical()
        .id_salt("album_grid")
        .auto_shrink([false, false])
        .show_rows(ui, tile_height + gap, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let Some(group) = groups.get(row * columns + column) else {
                            continue;
                        };
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(tile_width, tile_height),
                            egui::Sense::click(),
                        );
                        response.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Button,
                                true,
                                format!(
                                    "{} by {}, {} track(s)",
                                    group.album, group.artist, group.track_count
                                ),
                            )
                        });
                        let hover =
                            ui.ctx()
                                .animate_bool_with_time(response.id, response.hovered(), 0.12);
                        let tile = rect.translate(egui::vec2(0.0, -2.0 * hover));
                        if hover > 0.0 && !th::is_square() {
                            ui.painter().rect_filled(
                                tile,
                                th::corner(10),
                                th::alpha(th::hover_fill(), (hover * 255.0) as u8),
                            );
                        }

                        let cover = egui::Rect::from_min_size(
                            tile.min + egui::vec2(8.0, 8.0),
                            egui::vec2(tile_width - 16.0, tile_width - 16.0),
                        );
                        if !th::is_square() {
                            ui.painter().rect_filled(
                                cover.translate(egui::vec2(0.0, 3.0)),
                                th::corner(10),
                                Color32::from_black_alpha(if th::skin().is_light() {
                                    24
                                } else {
                                    60
                                }),
                            );
                        }
                        let cover_key = crate::app::album_key(&group.album, &group.artist);
                        let texture = crate::app::load_cover_texture(
                            &ctx,
                            &mut covers,
                            &cover_key,
                            std::path::Path::new(&group.path),
                        );
                        if let Some(texture) = texture {
                            egui::Image::new((texture.id(), cover.size()))
                                .corner_radius(th::corner(9))
                                .paint_at(ui, cover);
                        } else if covers.is_pending(&cover_key) {
                            // Decoding in the background: keep the tile calm
                            // instead of flashing "NO COVER".
                            ui.painter()
                                .rect_filled(cover, th::corner(9), th::bg_row_alt());
                            ui.painter().text(
                                cover.center(),
                                egui::Align2::CENTER_CENTER,
                                icons::MUSIC_NOTES.as_str(),
                                egui::FontId::proportional(28.0),
                                th::alpha(th::text_dim(), 90),
                            );
                        } else {
                            ui.painter()
                                .rect_filled(cover, th::corner(9), th::bg_row_alt());
                            ui.painter().text(
                                cover.center(),
                                egui::Align2::CENTER_CENTER,
                                "NO COVER",
                                egui::FontId::proportional(10.5),
                                th::text_dim(),
                            );
                        }
                        if !th::is_square() {
                            ui.painter().rect_stroke(
                                cover,
                                th::corner(9),
                                egui::Stroke::new(
                                    1.0,
                                    if hover > 0.0 {
                                        th::alpha(th::accent(), (hover * 150.0) as u8)
                                    } else {
                                        th::hairline()
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }

                        let mut labels = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(egui::Rect::from_min_max(
                                    egui::pos2(cover.left(), cover.bottom() + 9.0),
                                    egui::pos2(cover.right(), tile.bottom()),
                                ))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        labels.spacing_mut().item_spacing.y = 1.0;
                        labels.add(
                            egui::Label::new(th::semibold(&group.album, 12.5).color(th::text()))
                                .truncate()
                                .selectable(false),
                        );
                        labels.add(
                            egui::Label::new(
                                RichText::new(&group.artist)
                                    .size(11.0)
                                    .color(th::text_dim()),
                            )
                            .truncate()
                            .selectable(false),
                        );
                        labels.add(
                            egui::Label::new(
                                RichText::new(format!("{} track(s)", group.track_count))
                                    .size(9.5)
                                    .color(th::text_dim()),
                            )
                            .selectable(false),
                        );

                        if response.clicked() {
                            act = Some(Act::Open(group.album.clone()));
                        }
                        response.context_menu(|ui| {
                            if ui.button(tr("ui.open_album")).clicked() {
                                act = Some(Act::Open(group.album.clone()));
                                ui.close();
                            }
                            if ui.button(tr("ui.fetch_missing_cover")).clicked() {
                                act = Some(Act::Fetch(group.album.clone(), group.artist.clone()));
                                ui.close();
                            }
                            if ui.button(tr("ui.replace_cover")).clicked() {
                                let start = PathBuf::from(&group.path)
                                    .parent()
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| {
                                        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
                                    });
                                act = Some(Act::Replace(
                                    group.album.clone(),
                                    group.artist.clone(),
                                    start,
                                ));
                                ui.close();
                            }
                            if ui.button(tr("ui.save_cover_jpg")).clicked() {
                                act = Some(Act::Sidecar(group.album.clone(), group.artist.clone()));
                                ui.close();
                            }
                        });
                    }
                });
                ui.add_space(gap);
            }
        });

    app.album_covers = covers;

    match act {
        Some(Act::Open(album)) => app.open_album(&album),
        Some(Act::Fetch(album, artist)) => app.fetch_album_cover(&ctx, &album, &artist),
        Some(Act::Replace(album, artist, start)) => {
            app.open_cover_picker(&album, &artist, start);
        }
        Some(Act::Sidecar(album, artist)) => app.save_album_sidecar(&album, &artist),
        None => {}
    }
}

pub fn cover_picker(app: &mut App, ctx: &egui::Context) {
    let Some(picker) = &mut app.cover_picker else {
        return;
    };

    let album = picker.album.clone();
    let artist = picker.artist.clone();
    let mut open = true;
    let mut navigate: Option<PathBuf> = None;
    let mut choose: Option<PathBuf> = None;
    let mut cancel = false;

    egui::Window::new(tr("ui.choose_cover_image"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(540.0)
        .default_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr("ui.up")).clicked() {
                    if let Some(parent) = picker.current.parent().map(PathBuf::from) {
                        navigate = Some(parent);
                    }
                }
                if ui.button(tr("ui.home")).clicked() {
                    if let Some(home) = dirs::home_dir() {
                        navigate = Some(home);
                    }
                }
                ui.label(
                    RichText::new(format!("Cover for {}", picker.album))
                        .size(11.0)
                        .color(th::text_dim()),
                );
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
                    for dir in &picker.dirs {
                        let name = dir
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if ui.selectable_label(false, format!("📁 {name}")).clicked() {
                            navigate = Some(dir.clone());
                        }
                    }
                    for image in &picker.images {
                        let name = image
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let selected = picker.selected.as_deref() == Some(image.as_path());
                        let response = ui.selectable_label(selected, format!("🖼 {name}"));
                        if response.clicked() {
                            picker.selected = Some(image.clone());
                        }
                        if response.double_clicked() {
                            choose = Some(image.clone());
                        }
                    }
                    if picker.dirs.is_empty() && picker.images.is_empty() {
                        ui.label(
                            RichText::new("No sub-folders or images here.")
                                .size(11.0)
                                .color(th::text_dim()),
                        );
                    }
                    if let Some(error) = &picker.error {
                        ui.label(
                            RichText::new(error)
                                .size(10.5)
                                .color(Color32::from_rgb(220, 110, 110)),
                        );
                    }
                });

            ui.separator();
            let enabled = picker.selected.is_some();
            let (use_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, "Use this image", enabled);
            if use_clicked {
                choose = picker.selected.clone();
            }
            cancel |= cancel_clicked;
        });

    if let Some(dir) = navigate {
        if let Some(picker) = &mut app.cover_picker {
            if dir.is_dir() {
                picker.current = dir;
                picker.selected = None;
                picker.reload();
            }
        }
    } else if let Some(path) = choose {
        app.apply_cover_path(&album, &artist, &path);
    } else if cancel || !open {
        app.cover_picker = None;
    }
}

/// Small album thumbnail used by the album-art track layout.
fn track_art(
    ui: &mut egui::Ui,
    track: &Track,
    ctx: &egui::Context,
    covers: &mut crate::app::CoverCache,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
    let key = crate::app::track_cover_key(track);
    let texture =
        crate::app::load_cover_texture(ctx, covers, &key, std::path::Path::new(&track.path));
    if let Some(texture) = texture {
        egui::Image::new((texture.id(), rect.size()))
            .corner_radius(th::corner(5))
            .paint_at(ui, rect);
    } else {
        ui.painter()
            .rect_filled(rect, th::corner(5), th::bg_row_alt());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::MUSIC_NOTES.as_str(),
            egui::FontId::proportional(15.0),
            th::text_dim(),
        );
    }
}

/// Change the track-list sort, toggling direction when the key is already active.
fn apply_sort(app: &mut App, key: SortKey) {
    if app.sort == key {
        app.sort_asc = !app.sort_asc;
    } else {
        app.sort = key;
        app.sort_asc = true;
    }
    app.mark_tracks_dirty();
}

/// Five clickable stars. Returns the newly chosen rating (`0` clears it).
fn rating_stars(ui: &mut egui::Ui, rating: u8, interactive: bool, size: f32) -> Option<u8> {
    let mut selected = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for value in 1..=5u8 {
            let filled = rating >= value;
            let color = if filled {
                th::accent()
            } else {
                th::blend(th::text_dim(), th::bg_dark(), 0.55)
            };
            let mut button = egui::Button::new(icons::text(icons::STAR, size).color(color))
                .frame(false)
                .min_size(egui::vec2(size + 1.0, size + 1.0));
            if !interactive {
                button = button.sense(egui::Sense::hover());
            }
            let response = ui.add(button);
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Button,
                    interactive,
                    format!("Rate {value} star(s)"),
                )
            });
            if interactive
                && response
                    .on_hover_text(format!("Rate {value} star(s)"))
                    .clicked()
            {
                selected = Some(if rating == value { 0 } else { value });
            }
        }
    });
    selected
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

    let track_rows = std::mem::take(&mut app.tracks);
    let tracks: &[Track] = &track_rows;
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
    let mut rating_change: Option<(i64, u8)> = None;

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
                if icons::button_enabled(ui, has_selection, icons::TRASH, tr("common.remove"))
                    .on_hover_text("Remove the selection from this playlist (Delete)")
                    .clicked()
                {
                    ui_action = Some(UiAction::Remove);
                }
                if icons::compact_enabled(
                    ui,
                    has_selection,
                    icons::ARROW_UP,
                    tr("ui.move_up_alt_up"),
                )
                .clicked()
                {
                    ui_action = Some(UiAction::MoveUp);
                }
                if icons::compact_enabled(
                    ui,
                    has_selection,
                    icons::ARROW_DOWN,
                    tr("ui.move_down_alt_down"),
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
                tr("ui.no_tracks_match_your_search")
            } else if matches!(
                app.view,
                crate::app::View::Playlist(_) | crate::app::View::Smart(_)
            ) {
                tr("ui.this_view_is_empty_drag_tracks_here_from_the_library")
            } else {
                tr("ui.scan_a_music_folder_to_build_your_library")
            };
            ui.add_space(70.0);
            ui.vertical_centered(|ui| {
                ui.label(icons::text(icons::MUSIC_NOTES, 42.0).color(th::text_dim()));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("ui.nothing_here_yet"))
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

        let layout = app.settings.columns.layout;
        let visible_columns = app.settings.columns.visible();
        let art_column = layout == TrackLayout::AlbumArt;
        let compact = layout == TrackLayout::Compact;
        let row_height = match layout {
            TrackLayout::Standard => 26.0,
            TrackLayout::Compact => 19.0,
            TrackLayout::AlbumArt => 52.0,
        };
        let header_height = if compact { 22.0 } else { 26.0 };
        let title_size = if compact { 11.5 } else { 13.0 };
        let text_size = if compact { 11.0 } else { 12.0 };
        let small_size = if compact { 10.5 } else { 11.5 };

        // If the configured widths fit, keep them all. Otherwise shrink every
        // resizable column proportionally, down to its minimum, so the last
        // column (Rating) is never clipped off the right edge.
        let spacing = ui.spacing().item_spacing.x;
        let extra_columns = visible_columns.len() + usize::from(art_column) + 1;
        let configured_widths: Vec<f32> = visible_columns
            .iter()
            .map(|column| app.settings.columns.width(*column))
            .collect();
        let chrome = spacing * extra_columns as f32 + if art_column { 44.0 } else { 0.0 };
        let configured_total: f32 = configured_widths.iter().sum::<f32>() + chrome;
        let available = (ui.available_width() - 12.0).max(160.0);
        let shrunk = configured_total > available + 0.5;
        let mut column_widths = configured_widths.clone();
        if shrunk {
            let mut excess = configured_total - available;
            for _ in 0..3 {
                let slack: f32 = visible_columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| (column_widths[index] - column.min_width()).max(0.0))
                    .sum();
                if slack <= 0.5 {
                    break;
                }
                let take = (excess / slack).clamp(0.0, 1.0);
                for (index, column) in visible_columns.iter().enumerate() {
                    let minimum = column.min_width();
                    column_widths[index] -= (column_widths[index] - minimum).max(0.0) * take;
                }
                excess = column_widths.iter().sum::<f32>() + chrome - available;
                if excess <= 0.5 {
                    break;
                }
            }
        }

        // The table draws its own texture lookups for the album-art layout.
        // Take the cache out so the track slice can stay borrowed immutably.
        let mut covers = std::mem::take(&mut app.album_covers);
        let mut captured_widths: Vec<f32> = Vec::new();
        let table_id = format!(
            "tracks-{}-{}",
            match layout {
                TrackLayout::Standard => "standard",
                TrackLayout::Compact => "compact",
                TrackLayout::AlbumArt => "album-art",
            },
            visible_columns
                .iter()
                .map(|column| format!("{column:?}"))
                .collect::<Vec<_>>()
                .join("-"),
        );
        let row_painter = ui.painter().clone();
        let mut table = TableBuilder::new(ui)
            .id_salt(table_id)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click_and_drag())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        if app.columns_reset {
            table.reset();
        }
        if art_column {
            table = table.column(Column::exact(44.0).clip(true));
        }
        for (index, column) in visible_columns.iter().enumerate() {
            let width = column_widths[index];
            let minimum = column.min_width();
            table = table.column(Column::initial(width).at_least(minimum).clip(true));
        }
        // Absorb unused room when every configured width fits.
        table = table.column(Column::remainder().resizable(false));

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
            let text = th::semibold(format!("{label}{arrow}"), 10.5).color(if active {
                th::accent()
            } else {
                th::text_dim()
            });
            let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            if resp.clicked() {
                if let Some(k) = key {
                    new_sort = Some(k);
                }
            }
        };

        table
            .header(header_height, |mut h| {
                if art_column {
                    h.col(|ui| {
                        ui.label(icons::text(icons::IMAGE, 12.0).color(th::text_dim()))
                            .widget_info(|| {
                                WidgetInfo::labeled(WidgetType::Label, true, tr("ui.album_art"))
                            });
                    });
                }
                for column in &visible_columns {
                    h.col(|ui| {
                        let key = match column {
                            TrackColumn::Title => Some(SortKey::Title),
                            TrackColumn::Artist => Some(SortKey::Artist),
                            TrackColumn::Album => Some(SortKey::Album),
                            TrackColumn::Genre => Some(SortKey::Genre),
                            TrackColumn::Year => Some(SortKey::Year),
                            TrackColumn::Duration => Some(SortKey::Duration),
                            TrackColumn::Rating => Some(SortKey::Rating),
                            TrackColumn::Number | TrackColumn::Format => None,
                        };
                        header(ui, column.label(), key);
                    });
                }
                h.col(|_ui| {});
                let header_rect = h.response().rect;
                row_painter.hline(
                    header_rect.x_range(),
                    header_rect.bottom() - 0.5,
                    egui::Stroke::new(1.0, th::hairline()),
                );
            })
            .body(|body| {
                captured_widths = body.widths().to_vec();
                body.rows(row_height, tracks.len(), |mut row| {
                    let idx = row.index();
                    let track = &tracks[idx];
                    let is_sel = selected.contains(&track.id);
                    let is_playing = playing_id == Some(track.id);
                    row.set_selected(is_sel);

                    if art_column {
                        row.col(|ui| track_art(ui, track, &ctx, &mut covers));
                    }

                    for column in &visible_columns {
                        match column {
                            TrackColumn::Number => {
                                let (_, cell) = row.col(|ui| {
                                    if is_playing {
                                        ui.label(
                                            icons::text(icons::PLAY, 11.0).color(th::accent()),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!(
                                                "{:>2}",
                                                track.track_no.unwrap_or(0)
                                            ))
                                            .monospace()
                                            .size(small_size)
                                            .color(th::text_dim()),
                                        );
                                    }
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::TrackNo);
                                }
                            }
                            TrackColumn::Title => {
                                let (_, cell) = row.col(|ui| {
                                    let text = th::semibold(&track.title, title_size)
                                        .color(if is_playing { th::accent() } else { th::text() });
                                    ui.add(egui::Label::new(text).truncate());
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::Title);
                                }
                            }
                            TrackColumn::Artist => {
                                let (_, cell) = row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&track.artist)
                                                .size(text_size)
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::Artist);
                                }
                            }
                            TrackColumn::Album => {
                                let (_, cell) = row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&track.album)
                                                .size(text_size)
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::Album);
                                }
                            }
                            TrackColumn::Genre => {
                                let (_, cell) = row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&track.genre)
                                                .size(small_size)
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::Genre);
                                }
                            }
                            TrackColumn::Year => {
                                let (_, cell) = row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(
                                                track
                                                    .year
                                                    .map(|year| year.to_string())
                                                    .unwrap_or_default(),
                                            )
                                            .size(small_size)
                                            .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                                if cell.clicked() {
                                    cell_focus = Some(TrackField::Year);
                                }
                            }
                            TrackColumn::Duration => {
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(fmt_time(track.duration_secs as f64))
                                                .monospace()
                                                .size(small_size)
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                            }
                            TrackColumn::Rating => {
                                row.col(|ui| {
                                    if let Some(rating) = rating_stars(ui, track.rating, true, 13.0)
                                    {
                                        rating_change = Some((track.id, rating));
                                    }
                                });
                            }
                            TrackColumn::Format => {
                                row.col(|ui| {
                                    let label = if track.bitrate > 0 {
                                        format!("{}k", track.bitrate)
                                    } else {
                                        track.format.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(label)
                                                .size(if compact { 10.0 } else { 11.0 })
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                });
                            }
                        }
                    }

                    row.col(|_ui| {});

                    let resp = row.response();
                    if is_sel && !art_column {
                        let rect = resp.rect;
                        let bar = egui::Rect::from_min_size(
                            egui::pos2(rect.left() + 2.0, rect.top() + 5.0),
                            egui::vec2(3.0, (rect.height() - 10.0).max(4.0)),
                        );
                        row_painter.rect_filled(bar, th::pill(), th::accent());
                    }
                    if resp.secondary_clicked() && !is_sel {
                        act = Some(RowAct::Click {
                            index: idx,
                            ctrl: false,
                            shift: false,
                        });
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(icons::with(icons::PLAY, tr("ui.play_now")))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::PlayNow(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::SKIP_FORWARD, tr("ui.play_next")))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::PlayNext(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::QUEUE, tr("ui.add_to_queue")))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::AddToQueue(idx));
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(icons::with(icons::PENCIL_SIMPLE, tr("ui.rename")))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::Rename(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::TAG, tr("ui.edit_tags")))
                            .clicked()
                        {
                            menu_action = Some(MenuAction::EditTags(idx));
                            ui.close();
                        }
                        if ui
                            .button(icons::with(icons::MAGIC_WAND, tr("ui.find_metadata")))
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
                                    tr("ui.remove_from_playlist"),
                                )),
                            )
                            .on_disabled_hover_text("Only available inside a playlist")
                            .clicked()
                        {
                            menu_action = Some(MenuAction::RemoveFromPlaylist(idx));
                            ui.close();
                        }
                        ui.menu_button(icons::with(icons::PLUS, tr("ui.add_to_playlist")), |ui| {
                            if playlists.is_empty() {
                                ui.label(
                                    RichText::new(tr("ui.no_playlists_yet"))
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

        app.album_covers = covers;
        app.columns_reset = false;
        if !captured_widths.is_empty() {
            captured_widths.pop(); // trailing spacer
        }
        if art_column && !captured_widths.is_empty() {
            captured_widths.remove(0);
        }
        let mut persist_columns = Vec::new();
        let mut persist_widths = Vec::new();
        // Auto-shrunk widths are a viewport compromise, not a user choice:
        // keep the configured widths for when the window grows again.
        if !shrunk {
            for (index, column) in visible_columns.iter().enumerate() {
                if let Some(width) = captured_widths.get(index) {
                    persist_columns.push(*column);
                    persist_widths.push(*width);
                }
            }
        }
        app.update_column_widths(&persist_columns, &persist_widths);
    });

    app.tracks = track_rows;
    if let Some((track_id, rating)) = rating_change {
        app.set_track_rating(track_id, rating);
    }

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

fn batch_field_combo(ui: &mut egui::Ui, id: &str, field: &mut BatchField) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(field.label()).size(11.0))
        .width(120.0)
        .show_ui(ui, |ui| {
            for candidate in BatchField::ALL {
                ui.selectable_value(field, candidate, candidate.label());
            }
        });
}

pub fn tag_editor(app: &mut App, ctx: &egui::Context) {
    let Some(state) = &mut app.editing else {
        return;
    };

    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    let mut batch_action: Option<BatchAction> = None;

    let batch = state.batch;
    let count = state.paths.len();
    let title = if batch {
        format!("Batch properties · {count} tracks")
    } else {
        tr("ui.edit_tags").to_string()
    };
    let hint = if batch { "(unchanged)" } else { "" };

    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(470.0)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme())).inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 6,
                bottom: 6,
            }),
        )
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
                        let label = ui.label(RichText::new(label).size(12.0).color(th::text_dim()));
                        ui.add_sized(
                            [320.0, 22.0],
                            egui::TextEdit::singleline(value).hint_text(hint),
                        )
                        .labelled_by(label.id);
                        ui.end_row();
                    };
                    field(ui, tr("column.title"), &mut state.edit.title);
                    field(ui, tr("column.artist"), &mut state.edit.artist);
                    field(ui, "Album", &mut state.edit.album);
                    field(ui, "Album artist", &mut state.edit.album_artist);
                    field(ui, tr("column.genre"), &mut state.edit.genre);

                    let mut year = state.edit.year.unwrap_or(0);
                    let mut track_no = state.edit.track_no.unwrap_or(0);
                    let mut disc_no = state.edit.disc_no.unwrap_or(0);

                    let year_label = ui.label(
                        RichText::new(tr("column.year"))
                            .size(12.0)
                            .color(th::text_dim()),
                    );
                    ui.add(egui::DragValue::new(&mut year).range(0..=9999).speed(1.0))
                        .labelled_by(year_label.id);
                    ui.end_row();

                    let track_label =
                        ui.label(RichText::new("Track no").size(12.0).color(th::text_dim()));
                    ui.add(
                        egui::DragValue::new(&mut track_no)
                            .range(0..=999)
                            .speed(1.0),
                    )
                    .labelled_by(track_label.id);
                    ui.end_row();

                    let disc_label =
                        ui.label(RichText::new("Disc no").size(12.0).color(th::text_dim()));
                    ui.add(egui::DragValue::new(&mut disc_no).range(0..=99).speed(1.0))
                        .labelled_by(disc_label.id);
                    ui.end_row();

                    state.edit.year = (year > 0).then_some(year);
                    state.edit.track_no = (track_no > 0).then_some(track_no);
                    state.edit.disc_no = (disc_no > 0).then_some(disc_no);

                    let gain_label = ui.label(
                        RichText::new("Volume (ReplayGain)")
                            .size(12.0)
                            .color(th::text_dim()),
                    );
                    gain_label.on_hover_text(
                        "Per-track gain in dB, written as REPLAYGAIN_TRACK_GAIN.                          Playback applies it when Settings → Apply ReplayGain tags is on.",
                    );
                    let mut has_gain = state.edit.replay_gain.is_some();
                    let mut gain = state.edit.replay_gain.unwrap_or(0.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut has_gain, "");
                        ui.scope(|ui| {
                            ui.spacing_mut().slider_width = 180.0;
                            ui.add_enabled(
                                has_gain,
                                egui::Slider::new(&mut gain, -24.0..=24.0)
                                    .suffix(" dB")
                                    .step_by(0.1)
                                    .fixed_decimals(1),
                            );
                        });
                        if batch && !has_gain {
                            ui.label(
                                RichText::new("leave unchanged")
                                    .size(9.5)
                                    .color(th::text_dim()),
                            );
                        }
                    });
                    ui.end_row();
                    state.edit.replay_gain = has_gain.then_some(gain);

                    if batch {
                        ui.label(
                            RichText::new("0 keeps the existing number")
                                .size(9.0)
                                .color(th::text_dim()),
                        );
                        ui.end_row();
                    }
                });

            if batch {
                ui.add_space(8.0);
                ui.separator();
                ui.label(
                    RichText::new(tr("ui.batch_actions"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.label(
                    RichText::new(
                        "These read and write each file immediately; Undo restores the batch.",
                    )
                    .size(9.5)
                    .color(th::text_dim()),
                );
                ui.horizontal(|ui| {
                    if ui.button("Auto-number tracks").clicked() {
                        batch_action = Some(BatchAction::AutoNumber);
                    }
                    ui.label(RichText::new("Case").size(10.5).color(th::text_dim()));
                    egui::ComboBox::from_id_salt("batch_case")
                        .selected_text(RichText::new(app.batch_case.label()).size(11.0))
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for case in BatchCase::ALL {
                                ui.selectable_value(&mut app.batch_case, case, case.label());
                            }
                        });
                    if ui.button("Apply case").clicked() {
                        batch_action = Some(BatchAction::Case(app.batch_case));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr("common.clear"))
                            .size(10.5)
                            .color(th::text_dim()),
                    );
                    batch_field_combo(ui, "batch_clear", &mut app.batch_field);
                    if ui.button("Clear field").clicked() {
                        batch_action = Some(BatchAction::Clear(app.batch_field));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Copy field from the first track")
                            .size(10.5)
                            .color(th::text_dim()),
                    );
                    batch_field_combo(ui, "batch_copy", &mut app.batch_copy);
                    if ui.button("Copy").clicked() {
                        batch_action = Some(BatchAction::Copy(app.batch_copy));
                    }
                });
            }

            if let Some(status) = &state.status {
                ui.add_space(4.0);
                ui.label(RichText::new(status).size(11.0).color(th::accent()));
            }

            ui.add_space(8.0);
            ui.label(
                RichText::new(if batch {
                    "Only filled fields are written"
                } else {
                    "Writes tags into the file itself"
                })
                .size(10.0)
                .color(th::text_dim()),
            );
            ui.add_space(4.0);
            let save_label = if batch {
                format!("Save to {count} files")
            } else {
                tr("common.save").to_string()
            };
            let (save_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, &save_label, true);
            save |= save_clicked;
            cancel |= cancel_clicked;
        });

    if !open {
        cancel = true;
    }
    if let Some(action) = batch_action {
        app.apply_batch_action(action);
    }
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

    let mut open = true;
    let mut commit = false;
    let mut cancel = false;
    let title = icons::with(icons::PENCIL_SIMPLE, &state.target.title());

    egui::Window::new(title)
        .open(&mut open)
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
            let hint_label = ui.label(RichText::new(&hint).size(10.5).color(th::text_dim()));
            ui.add_space(4.0);

            let resp = ui
                .add_sized(
                    [300.0, 24.0],
                    egui::TextEdit::singleline(&mut state.text).hint_text("new value"),
                )
                .labelled_by(hint_label.id);
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
            ui.label(
                RichText::new("Enter saves, Escape cancels")
                    .size(10.0)
                    .color(th::text_dim()),
            );
            ui.add_space(4.0);
            let (save_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, tr("common.save"), true);
            commit |= save_clicked;
            cancel |= cancel_clicked;

            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });

    if !open {
        cancel = true;
    }
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
    let mut empty = app.playlist_dialog_empty;

    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    egui::Window::new(tr("ui.new_playlist"))
        .open(&mut open)
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
                RichText::new(if empty {
                    "Create an empty playlist; add tracks later by dragging them in.".to_string()
                } else if selected > 0 {
                    format!("{selected} selected track(s) will be saved")
                } else {
                    format!("all {total} tracks in this view will be saved")
                })
                .size(11.0)
                .color(th::text_dim()),
            );
            if let Some(folder) = app.playlist_dialog_folder {
                let path = app.playlist_folder_path(folder);
                if !path.is_empty() {
                    ui.label(
                        RichText::new(format!("Destination: {path}"))
                            .size(10.5)
                            .color(th::accent()),
                    );
                }
            }
            ui.add_space(4.0);
            ui.checkbox(
                &mut empty,
                RichText::new(tr("ui.create_empty_playlist")).size(11.0),
            )
            .on_hover_text(tr("ui.do_not_copy_the_current_selection_or_view"));
            ui.add_space(4.0);
            let name_label = ui.label(RichText::new(tr("ui.playlist_name")).size(10.5));
            let resp = ui
                .add_sized(
                    [260.0, 22.0],
                    egui::TextEdit::singleline(&mut name).hint_text("playlist name"),
                )
                .labelled_by(name_label.id);
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                create = true;
            }
            ui.add_space(6.0);
            let (create_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, tr("common.create"), true);
            create |= create_clicked;
            cancel |= cancel_clicked;
        });

    if !open {
        cancel = true;
    }
    app.playlist_dialog_empty = empty;
    app.playlist_dialog = Some(name);
    if create {
        app.commit_playlist_dialog();
    } else if cancel {
        app.playlist_dialog = None;
        app.playlist_dialog_empty = false;
        app.playlist_dialog_folder = None;
    }
}

/// Create a new playlist folder (optionally inside another folder).
pub fn folder_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = app.folder_dialog.clone() else {
        return;
    };

    let parent = draft
        .parent_id
        .map(|id| app.playlist_folder_path(id))
        .filter(|path| !path.is_empty());
    let title = parent
        .as_ref()
        .map(|path| format!("New folder in {path}"))
        .unwrap_or_else(|| tr("ui.new_playlist_folder").to_string());

    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            let name_label = ui.label(RichText::new(tr("ui.folder_name")).size(10.5));
            let resp = ui
                .add_sized(
                    [280.0, 22.0],
                    egui::TextEdit::singleline(&mut draft.name).hint_text("folder name"),
                )
                .labelled_by(name_label.id);
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                create = true;
            }
            if let Some(error) = &draft.status {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} {error}", icons::WARNING.as_str()))
                        .size(10.5)
                        .color(Color32::from_rgb(230, 120, 120)),
                );
            }
            ui.add_space(6.0);
            let (create_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, tr("common.create"), true);
            create |= create_clicked;
            cancel |= cancel_clicked;
        });

    if !open {
        cancel = true;
    }
    if create {
        app.folder_dialog = Some(draft);
        app.commit_folder_dialog();
    } else if cancel {
        app.folder_dialog = None;
    } else {
        app.folder_dialog = Some(draft);
    }
}

/// Import an M3U file or export the open playlist to one.
pub fn m3u_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = app.m3u_dialog.clone() else {
        return;
    };

    let importing = matches!(draft.mode, M3uMode::Import);
    let title = if importing {
        "Import M3U playlist".to_string()
    } else {
        format!("Export \"{}\" as M3U", draft.name)
    };
    let mut open = true;
    let mut commit = false;
    let mut cancel = false;

    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(540.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(if importing {
                    "Relative paths in the file are resolved beside it. Missing audio is skipped."
                } else {
                    "The playlist is written with absolute track paths and #EXTINF metadata."
                })
                .size(10.5)
                .color(th::text_dim()),
            );
            ui.add_space(6.0);

            let file_label = ui.label(RichText::new(tr("ui.playlist_file")).size(10.5));
            let path_resp = ui
                .add_sized(
                    [520.0, 22.0],
                    egui::TextEdit::singleline(&mut draft.path).hint_text(if importing {
                        "/home/me/Music/mix.m3u8"
                    } else {
                        "/home/me/Music/playlist.m3u8"
                    }),
                )
                .labelled_by(file_label.id);

            if importing {
                ui.add_space(6.0);
                let name_label = ui.label(RichText::new(tr("ui.playlist_name")).size(10.5));
                ui.add_sized(
                    [320.0, 22.0],
                    egui::TextEdit::singleline(&mut draft.name)
                        .hint_text("optional, defaults to the file name"),
                )
                .labelled_by(name_label.id);

                ui.add_space(6.0);
                ui.label(RichText::new(tr("ui.folder")).size(10.5));
                let selected = draft
                    .folder_id
                    .map(|id| app.playlist_folder_path(id))
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| tr("ui.top_level").to_string());
                egui::ComboBox::from_id_salt("m3u_folder")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.folder_id, None, tr("ui.top_level"));
                        for folder in &app.playlist_folders {
                            let label = app.playlist_folder_path(folder.id);
                            ui.selectable_value(&mut draft.folder_id, Some(folder.id), label);
                        }
                    });
            }

            if let Some(error) = &draft.status {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{} {error}", icons::WARNING.as_str()))
                        .size(10.5)
                        .color(Color32::from_rgb(230, 120, 120)),
                );
            }

            ui.add_space(8.0);
            let label = icons::with(
                if importing {
                    icons::DOWNLOAD_SIMPLE
                } else {
                    icons::UPLOAD_SIMPLE
                },
                if importing {
                    tr("ui.import")
                } else {
                    tr("ui.export")
                },
            );
            let (commit_clicked, cancel_clicked) = crate::dialog::save_cancel(ui, &label, true);
            commit |= commit_clicked;
            cancel |= cancel_clicked;
            if path_resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
        });

    if !open {
        cancel = true;
    }
    if commit {
        app.m3u_dialog = Some(draft);
        app.commit_m3u_dialog();
    } else if cancel {
        app.m3u_dialog = None;
    } else {
        app.m3u_dialog = Some(draft);
    }
}

/// Track-list column order, visibility, widths reset and row layout.
pub fn columns_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_columns {
        return;
    }
    let mut open = true;
    let mut close = false;
    let mut config = app.settings.columns.clone();
    let mut reset = false;

    egui::Window::new(tr("menu.track_columns"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(tr("ui.layout"))
                    .size(10.5)
                    .color(th::text_dim()),
            );
            ui.horizontal(|ui| {
                for layout in TrackLayout::ALL {
                    ui.selectable_value(&mut config.layout, layout, layout.label());
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Compact rows are denser; album art adds a cover thumbnail per row.")
                    .size(10.0)
                    .color(th::text_dim()),
            );
            ui.separator();

            ui.label(
                RichText::new(tr("ui.columns"))
                    .size(10.5)
                    .color(th::text_dim()),
            );
            ui.add_space(2.0);
            let order = config.normalized_order();
            for (index, column) in order.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut visible = !config.is_hidden(*column);
                    let can_hide = config.visible().len() > 1 || visible;
                    if ui
                        .add_enabled(can_hide, egui::Checkbox::new(&mut visible, column.label()))
                        .changed()
                    {
                        if visible {
                            config.hidden.retain(|hidden| hidden != column);
                        } else if !config.hidden.contains(column) {
                            config.hidden.push(*column);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::compact_enabled(
                            ui,
                            index + 1 < order.len(),
                            icons::ARROW_DOWN,
                            "Move column down",
                        )
                        .clicked()
                        {
                            config.move_column(*column, 1);
                        }
                        if icons::compact_enabled(ui, index > 0, icons::ARROW_UP, "Move column up")
                            .clicked()
                        {
                            config.move_column(*column, -1);
                        }
                    });
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(icons::with(
                        icons::ARROW_COUNTER_CLOCKWISE,
                        tr("ui.reset_columns"),
                    ))
                    .clicked()
                {
                    reset = true;
                }
                ui.label(
                    RichText::new("drag column dividers in the table to resize")
                        .size(9.5)
                        .color(th::text_dim()),
                );
            });
            ui.add_space(8.0);
            close = crate::dialog::close_row(ui);
        });

    if close {
        open = false;
    }
    if reset {
        config.reset();
    }
    if config != app.settings.columns {
        app.settings.columns = config;
        app.columns_reset = true;
        app.persist_settings();
    }
    if !open {
        app.show_columns = false;
    }
}

/// Create or edit a named search.
pub fn saved_search_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = app.saved_search_dialog.clone() else {
        return;
    };

    let editing = draft.id.is_some();
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    egui::Window::new(if editing {
        tr("ui.edit_saved_search")
    } else {
        tr("ui.save_search")
    })
    .open(&mut open)
    .collapsible(false)
    .resizable(false)
    .default_width(460.0)
    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    .show(ctx, |ui| {
        let name_label = ui.label(RichText::new(tr("ui.name")).size(10.5));
        let name_resp = ui
            .add_sized(
                [440.0, 22.0],
                egui::TextEdit::singleline(&mut draft.name).hint_text("e.g. Radiohead 1997"),
            )
            .labelled_by(name_label.id);
        ui.add_space(6.0);
        let query_label = ui.label(RichText::new(tr("ui.query")).size(10.5));
        let query_resp = ui
            .add_sized(
                [440.0, 22.0],
                egui::TextEdit::singleline(&mut draft.query)
                    .hint_text("artist:\"radiohead\" year:1997"),
            )
            .labelled_by(query_label.id);
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Fields: title, artist, album, albumartist, genre, format, \
                 year, track, disc, duration.",
            )
            .size(10.0)
            .color(th::text_dim()),
        );

        if let Some(error) = &draft.status {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("{} {error}", icons::WARNING.as_str()))
                    .size(10.5)
                    .color(Color32::from_rgb(230, 120, 120)),
            );
        }

        ui.add_space(8.0);
        let (save_clicked, cancel_clicked) =
            crate::dialog::save_cancel(ui, tr("common.save"), true);
        save |= save_clicked;
        cancel |= cancel_clicked;
        if (name_resp.has_focus() || query_resp.has_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            save = true;
        }
    });

    if !open {
        cancel = true;
    }
    if save {
        app.saved_search_dialog = Some(draft);
        app.commit_saved_search_dialog();
    } else if cancel {
        app.saved_search_dialog = None;
    } else {
        app.saved_search_dialog = Some(draft);
    }
}

pub fn settings_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }

    let mut open = true;
    let mut close = false;
    let mut reset = false;
    let mut unbind: Option<crate::settings::Action> = None;
    let mut rebind: Option<crate::settings::Action> = None;
    let mut chosen_skin: Option<crate::theme::Skin> = None;
    let mut open_skins = false;
    let mut reload_skins = false;
    let mut duplicate_skin = false;
    let mut edit_skin = false;
    let mut mini_player = app.settings.mini_player;
    let mut backup_library = false;
    let mut open_restore = false;
    #[cfg(target_os = "linux")]
    let mut tray_enabled = app.settings.tray_enabled;
    let mut chosen_language: Option<Language> = None;
    let mut chosen_device: Option<Option<String>> = None;
    let mut scrobble_enabled = app.settings.scrobble_listenbrainz;
    let mut local_feeds = app.settings.allow_local_feeds;
    let mut scrobble_token = app.settings.listenbrainz_token.clone();
    let mut crossfade_value: Option<f32> = None;
    let mut viz_enabled = app.settings.visualizer.enabled;
    let mut viz_mode = app.settings.visualizer.mode;
    let mut viz_preset: Option<String> = None;
    let mut viz_interval: Option<u32> = None;
    let mut viz_fps: Option<u32> = None;
    let mut viz_next = false;
    let mut viz_fullscreen = false;
    let conflicts = app.settings.conflicts();
    let mut tab = app.settings_tab;

    egui::Window::new(tr("settings.title"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab, SettingsTab::General, tr("settings.general"));
                ui.selectable_value(
                    &mut tab,
                    SettingsTab::LookAndFeel,
                    tr("settings.look_and_feel"),
                );
                ui.selectable_value(
                    &mut tab,
                    SettingsTab::Tracks,
                    tr("settings.track_options"),
                );
                ui.selectable_value(&mut tab, SettingsTab::Visualizer, "Visualizer");
                ui.selectable_value(&mut tab, SettingsTab::Keybindings, tr("settings.keybindings"));
                ui.selectable_value(&mut tab, SettingsTab::About, tr("settings.about"));
            });
            ui.separator();
            ui.add_space(4.0);

            if tab == SettingsTab::General {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("settings.language_section"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt("language_select")
                    .selected_text(RichText::new(app.settings.language.label()).size(12.0))
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for language in Language::ALL {
                            if ui
                                .selectable_label(
                                    app.settings.language == language,
                                    language.label(),
                                )
                                .clicked()
                            {
                                chosen_language = Some(language);
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("settings.accessibility_section"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                let mut large_fonts = app.settings.large_fonts;
                if ui
                    .checkbox(
                        &mut large_fonts,
                        RichText::new(tr("ui.larger_text_and_controls")).size(11.5),
                    )
                    .on_hover_text("Scales the whole interface, including tables and dialogs")
                    .changed()
                {
                    app.set_large_fonts(ctx, large_fonts);
                }
                ui.label(
                    RichText::new(
                        "Use Tab and Shift+Tab to move through controls; tooltips describe icon-only buttons.",
                    )
                    .size(10.0)
                    .color(th::text_dim()),
                );

            }

            if tab == SettingsTab::LookAndFeel {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("settings.skin_section"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt("skin_select")
                    .selected_text(RichText::new(app.settings.skin.label()).size(12.0))
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for skin in crate::theme::Skin::all() {
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
                    RichText::new(tr("ui.skins_apply_immediately_and_are_remembered_in_settings_json"))
                        .size(10.5)
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Open skins folder").clicked() {
                        open_skins = true;
                    }
                    if ui.small_button("Reload skins").clicked() {
                        reload_skins = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Duplicate selected skin").clicked() {
                        duplicate_skin = true;
                    }
                    if ui.small_button("Edit colours…").clicked() {
                        edit_skin = true;
                    }
                });

                ui.add_space(8.0);
                ui.label(
                    RichText::new("WINDOW")
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut mini_player, "Mini player (always on top)");
                #[cfg(target_os = "linux")]
                ui.checkbox(&mut tray_enabled, "Tray icon");

            }

            if tab == SettingsTab::Tracks {

                ui.label(
                    RichText::new(tr("library.section"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                scan_controls(app, ui);
                if let Some(progress) = app.scan_progress() {
                    let fraction = if progress.total > 0 {
                        (progress.found as f32 / progress.total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let bar = ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(190.0)
                            .text(
                                RichText::new(format!(
                                    "{} / {} files",
                                    progress.found, progress.total
                                ))
                                .size(9.5),
                            ),
                    );
                    if let Some(label) = app.scan_progress_label() {
                        bar.on_hover_text(label);
                    }
                    if progress.total > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{} indexed · {} unchanged",
                                progress.indexed, progress.unchanged
                            ))
                            .size(9.5)
                            .color(th::text_dim()),
                        );
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Back up library").clicked() {
                        backup_library = true;
                    }
                    if ui.small_button("Restore…").clicked() {
                        open_restore = true;
                    }
                });
                ui.add_space(8.0);
                let mut watching = app.settings.watch_enabled;
                if ui
                    .checkbox(
                        &mut watching,
                        RichText::new(tr("ui.watch_scanned_folders_for_new_or_changed_files")).size(11.5),
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

                ui.add_space(8.0);
                let mut replaygain = app.settings.replaygain;
                let mut album_gain = app.settings.replaygain_album;
                ui.checkbox(
                    &mut replaygain,
                    RichText::new(tr("ui.apply_replaygain_tags")).size(11.5),
                )
                .on_hover_text(
                    "Uses REPLAYGAIN_TRACK_GAIN / REPLAYGAIN_ALBUM_GAIN from the file when present.",
                );
                ui.add_enabled_ui(replaygain, |ui| {
                    ui.checkbox(
                        &mut album_gain,
                        RichText::new(tr("ui.prefer_album_gain")).size(11.0),
                    );
                });
                if replaygain != app.settings.replaygain
                    || album_gain != app.settings.replaygain_album
                {
                    app.set_replaygain(replaygain, album_gain);
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("settings.output_section"))
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                let selected = app
                    .settings
                    .output_device
                    .clone()
                    .unwrap_or_else(|| "System default".to_string());
                egui::ComboBox::from_id_salt("output_device")
                    .selected_text(RichText::new(selected).size(11.5))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                app.settings.output_device.is_none(),
                                "System default",
                            )
                            .clicked()
                        {
                            chosen_device = Some(None);
                        }
                        for name in &app.output_devices {
                            if ui
                                .selectable_label(
                                    app.settings.output_device.as_deref() == Some(name),
                                    name,
                                )
                                .clicked()
                            {
                                chosen_device = Some(Some(name.clone()));
                            }
                        }
                    });
                let mut crossfade = app.settings.crossfade_secs;
                if ui
                    .add(
                        egui::Slider::new(&mut crossfade, 0.0..=12.0)
                            .text(tr("ui.crossfade"))
                            .suffix(" s"),
                    )
                    .on_hover_text("0 s keeps gapless joining; 1–12 s overlaps tracks")
                    .changed()
                {
                    crossfade_value = Some(crossfade);
                }

                ui.add_space(8.0);
                ui.checkbox(
                    &mut scrobble_enabled,
                    RichText::new(tr("ui.scrobble_to_listenbrainz")).size(11.5),
                );
                if scrobble_enabled {
                    let token_label =
                        ui.label(RichText::new("ListenBrainz token").size(10.5));
                    ui.add_sized(
                        [300.0, 22.0],
                        egui::TextEdit::singleline(&mut scrobble_token)
                            .password(true)
                            .hint_text("ListenBrainz user token"),
                    )
                    .labelled_by(token_label.id);
                    ui.label(
                        RichText::new(
                            "No developer API key: paste a token from your ListenBrainz profile.",
                        )
                        .size(9.5)
                        .color(th::text_dim()),
                    );
                }

                ui.add_space(8.0);
                ui.checkbox(
                    &mut local_feeds,
                    RichText::new("Allow podcasts from local network addresses").size(11.5),
                );
                ui.label(
                    RichText::new(
                        "Off: feeds and episodes may only use public addresses. Turn on only for a \
                         podcast server on your own network; a feed you subscribe to can otherwise \
                         reach local services.",
                    )
                    .size(9.5)
                    .color(th::text_dim()),
                );
                        }

            if tab == SettingsTab::Visualizer {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("MILKDROP VISUALIZER")
                        .size(10.0)
                        .strong()
                        .color(th::text_dim()),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut viz_enabled, "Enable visualizer");
                ui.label(
                    RichText::new(
                        "Shows a panel in the library view and behind the Now Playing screen.",
                    )
                    .size(10.0)
                    .color(th::text_dim()),
                );
                if let Some(error) = &app.viz_error {
                    ui.label(
                        RichText::new(error.as_str())
                            .size(10.0)
                            .color(Color32::from_rgb(220, 110, 110)),
                    );
                }

                ui.add_space(10.0);
                ui.label(RichText::new("Preset mode").size(11.0).color(th::text()));
                ui.radio_value(
                    &mut viz_mode,
                    crate::settings::VisualizerMode::Selected,
                    "Selected preset",
                );
                ui.radio_value(
                    &mut viz_mode,
                    crate::settings::VisualizerMode::Shuffle,
                    "Shuffle all presets",
                );

                if viz_mode == crate::settings::VisualizerMode::Selected {
                    let presets: Vec<String> = app
                        .visualizer
                        .as_ref()
                        .map(|visualizer| {
                            visualizer
                                .presets()
                                .iter()
                                .map(|preset| preset.name.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    if presets.is_empty() {
                        ui.label(
                            RichText::new("No presets loaded. Enable the visualizer first.")
                                .size(10.0)
                                .color(th::text_dim()),
                        );
                    } else {
                        let selected = if app.settings.visualizer.preset.is_empty() {
                            presets[0].clone()
                        } else {
                            app.settings.visualizer.preset.clone()
                        };
                        egui::ComboBox::from_id_salt("viz_preset")
                            .selected_text(selected)
                            .width(240.0)
                            .show_ui(ui, |ui| {
                                for name in &presets {
                                    let is_selected = name == &app.settings.visualizer.preset;
                                    if ui.selectable_label(is_selected, name).clicked() {
                                        viz_preset = Some(name.clone());
                                    }
                                }
                            });
                    }
                } else {
                    let mut interval = app.settings.visualizer.interval_secs;
                    if ui
                        .add(
                            egui::Slider::new(&mut interval, 10..=180)
                                .text("seconds between presets"),
                        )
                        .changed()
                    {
                        viz_interval = Some(interval);
                    }
                }

                ui.add_space(8.0);
                let mut fps = app.settings.visualizer.fps;
                egui::ComboBox::from_id_salt("viz_fps")
                    .selected_text(format!("{fps} fps"))
                    .show_ui(ui, |ui| {
                        for value in [30, 60] {
                            if ui
                                .selectable_label(fps == value, format!("{value} fps"))
                                .clicked()
                            {
                                fps = value;
                            }
                        }
                    });
                if fps != app.settings.visualizer.fps {
                    viz_fps = Some(fps);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Next preset").clicked() {
                        viz_next = true;
                    }
                    if ui.button("Fullscreen").clicked() {
                        viz_fullscreen = true;
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Drop extra .milk / .milk2 files into the presets folder in the data directory.",
                    )
                    .size(9.5)
                    .color(th::text_dim()),
                );
            }

            if tab == SettingsTab::Keybindings {
                ui.label(
                    RichText::new(tr("ui.keyboard_shortcuts"))
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
                        "A native music library and player for Linux and Windows.",
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
                    if ui.button(tr("ui.reset_to_defaults")).clicked() {
                        reset = true;
                    }
                    ui.label(
                        RichText::new("Language, appearance and session are reset.")
                            .size(10.0)
                            .color(th::text_dim()),
                    );
                });
            }
            ui.add_space(8.0);
            close = crate::dialog::close_row(ui);
        });

    if close {
        open = false;
    }
    app.settings_tab = tab;

    if app.captured_this_frame {
        rebind = None;
    }
    app.captured_this_frame = false;

    if let Some(device) = chosen_device {
        app.set_output_device(device);
    }
    if let Some(secs) = crossfade_value {
        app.set_crossfade(secs);
    }
    if local_feeds != app.settings.allow_local_feeds {
        app.settings.allow_local_feeds = local_feeds;
        app.persist_settings();
    }
    if scrobble_enabled != app.settings.scrobble_listenbrainz
        || scrobble_token != app.settings.listenbrainz_token
    {
        if scrobble_token.trim().is_empty() {
            // Clearing the field must also clear the credential store, or the
            // token would come back on the next launch.
            if let Err(error) = musicwasp_core::secrets::delete_listenbrainz_token() {
                app.message = format!("could not remove the ListenBrainz token: {error:#}");
            }
        }
        app.settings.scrobble_listenbrainz = scrobble_enabled;
        app.settings.listenbrainz_token = scrobble_token;
        app.persist_settings();
    }
    if backup_library {
        if let Err(error) = app.backup_library() {
            app.message = format!("backup failed: {error}");
        }
    }
    if open_restore {
        app.open_restore_dialog();
    }
    if mini_player != app.settings.mini_player {
        app.set_mini_player(mini_player, ctx);
    }
    #[cfg(target_os = "linux")]
    if tray_enabled != app.settings.tray_enabled {
        app.set_tray_enabled(tray_enabled, ctx);
    }
    if viz_enabled != app.settings.visualizer.enabled {
        app.set_visualizer_enabled(viz_enabled, ctx);
    }
    if viz_mode != app.settings.visualizer.mode {
        app.set_visualizer_mode(viz_mode);
    }
    if let Some(name) = viz_preset {
        app.select_visualizer_preset(&name);
    }
    if let Some(interval) = viz_interval {
        app.settings.visualizer.interval_secs = interval.clamp(10, 180);
        app.persist_settings();
    }
    if let Some(fps) = viz_fps {
        app.settings.visualizer.fps = fps.clamp(15, 60);
        app.persist_settings();
    }
    if viz_next {
        app.next_visualizer_preset();
    }
    if viz_fullscreen {
        app.ensure_visualizer();
        app.viz_fullscreen = true;
    }
    if open_skins {
        app.open_skins_folder();
    }
    if reload_skins {
        app.reload_skins(ctx);
    }
    if duplicate_skin {
        app.duplicate_skin(ctx);
    }
    if edit_skin {
        app.open_skin_editor();
    }
    if let Some(skin) = chosen_skin {
        app.set_skin(ctx, skin);
    }
    if let Some(language) = chosen_language {
        app.set_language(language);
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
    let mut cancel = false;
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
                if ui.button(tr("ui.up")).clicked() {
                    if let Some(parent) = picker.current.parent().map(PathBuf::from) {
                        navigate = Some(parent);
                    }
                }
                if ui.button(tr("ui.home")).clicked() {
                    if let Some(home) = dirs::home_dir() {
                        navigate = Some(home);
                    }
                }
                if ui
                    .checkbox(&mut picker.show_hidden, tr("ui.hidden_folders"))
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
            ui.label(
                RichText::new(tr("ui.music_folders_are_scanned_recursively"))
                    .size(10.0)
                    .color(th::text_dim()),
            );
            let (choose_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, tr("ui.use_this_folder"), true);
            choose |= choose_clicked;
            cancel |= cancel_clicked;
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
    if cancel || !open {
        app.picker = None;
    }
}

pub fn autotag_window(app: &mut App, ctx: &egui::Context) {
    use crate::app::AutoTagStage;

    let undo_available = app.tag_undo.is_some();
    let has_acoustid = !app.effective_acoustid_key().is_empty();
    let Some(state) = &mut app.autotag else {
        return;
    };
    let targets = state.targets.len();

    let mut open = true;
    let mut search = false;
    let mut fingerprint = false;
    let mut apply = false;
    let mut undo = false;
    let mut reload_release = false;

    egui::Window::new(tr("ui.auto_tag_by_album"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(960.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("column.artist")).size(12.0).color(th::text_dim()));
                ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(&mut state.artist));
                ui.label(RichText::new("Album").size(12.0).color(th::text_dim()));
                ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(&mut state.album));
                if ui
                    .add_enabled(
                        !matches!(
                            state.stage,
                            AutoTagStage::Searching
                                | AutoTagStage::Fingerprinting
                                | AutoTagStage::Applying
                        ),
                        egui::Button::new(tr("ui.search")),
                    )
                    .clicked()
                {
                    search = true;
                }
                let identify = ui.add_enabled(
                    !matches!(
                        state.stage,
                        AutoTagStage::Searching
                            | AutoTagStage::Fingerprinting
                            | AutoTagStage::Applying
                    ) && has_acoustid,
                    egui::Button::new(icons::with(icons::FINGERPRINT, "Identify with AcoustID")),
                );
                if identify
                    .on_hover_text(if has_acoustid {
                        "Fingerprint one track with Chromaprint and look up the release on AcoustID"
                    } else {
                        "This build has no AcoustID key; set ACCOUSTID_APIKEY and rebuild"
                    })
                    .clicked()
                {
                    fingerprint = true;
                }
            });

            ui.add_space(6.0);

            match &state.stage {
                AutoTagStage::Editing => {
                    ui.label(
                        RichText::new(format!(
                            "{targets} track(s) will be tagged. Search MusicBrainz by name, or identify by audio fingerprint with AcoustID."
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
                AutoTagStage::Fingerprinting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Fingerprinting with Chromaprint and asking AcoustID…")
                                .size(12.0),
                        );
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

                            egui_extras::TableBuilder::new(ui)
                                .id_salt("autotag_mapping")
                                .striped(true)
                                .resizable(false)
                                .max_scroll_height(280.0)
                                .column(
                                    Column::initial(360.0).at_least(180.0).clip(true),
                                )
                                .column(
                                    Column::initial(300.0).at_least(220.0).clip(true),
                                )
                                .column(
                                    Column::remainder().at_least(180.0).clip(true),
                                )
                                .header(20.0, |mut header| {
                                    header.col(|ui| {
                                        ui.label(
                                            RichText::new("File")
                                                .size(10.0)
                                                .strong()
                                                .color(th::text_dim()),
                                        );
                                    });
                                    header.col(|ui| {
                                        ui.label(
                                            RichText::new("Release track")
                                                .size(10.0)
                                                .strong()
                                                .color(th::text_dim()),
                                        );
                                    });
                                    header.col(|ui| {
                                        ui.label(
                                            RichText::new("Changes")
                                                .size(10.0)
                                                .strong()
                                                .color(th::text_dim()),
                                        );
                                    });
                                })
                                .body(|mut body| {
                                    for mapping in &mut state.mappings {
                                        body.row(24.0, |mut row| {
                                            row.col(|ui| {
                                                let file_label =
                                                    if mapping.track.title.is_empty() {
                                                        PathBuf::from(&mapping.track.path)
                                                            .file_name()
                                                            .map(|name| {
                                                                name.to_string_lossy().to_string()
                                                            })
                                                            .unwrap_or_else(|| {
                                                                mapping.track.path.clone()
                                                            })
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
                                            });

                                            row.col(|ui| {
                                                let selected = mapping
                                                    .release_index
                                                    .and_then(|index| {
                                                        release.tracks.get(index)
                                                    })
                                                    .map(|track| {
                                                        format!(
                                                            "{}.{:02}  {}",
                                                            track.disc,
                                                            track.number,
                                                            track.title
                                                        )
                                                    })
                                                    .unwrap_or_else(|| {
                                                        "— skip —".to_string()
                                                    });
                                                egui::ComboBox::from_id_salt((
                                                    "autotag_map",
                                                    mapping.track.id,
                                                ))
                                                .selected_text(
                                                    RichText::new(selected).size(11.0),
                                                )
                                                .width((ui.available_width() - 4.0).max(80.0))
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
                                            });

                                            row.col(|ui| {
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
                                                        RichText::new(summary)
                                                            .size(10.5)
                                                            .color(if changes.is_empty() {
                                                                th::text_dim()
                                                            } else {
                                                                th::accent()
                                                            }),
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
                                            });
                                        });
                                    }
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
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let response = crate::dialog::primary_button(
                                        ui,
                                        &format!("Apply to {mapped} track(s)"),
                                        can_apply,
                                    )
                                    .on_disabled_hover_text(
                                        "Wait for the release track list and map at least one file",
                                    );
                                    if response.clicked() {
                                        apply = true;
                                    }
                                },
                            );
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
                    if undo_available
                        && ui
                            .button(icons::with(
                                icons::ARROW_COUNTER_CLOCKWISE,
                                "Undo last tag write",
                            ))
                            .clicked()
                    {
                        undo = true;
                    }
                }
            }
        });

    if search {
        app.start_autotag_search();
    }
    if fingerprint {
        app.start_autotag_fingerprint();
    }
    if reload_release {
        app.load_autotag_release();
    }
    if apply {
        app.start_autotag_apply();
    }
    if undo {
        app.undo_last_tag_write();
    }
    if !open {
        app.autotag = None;
    }
}

pub fn smart_dialog(app: &mut App, ctx: &egui::Context) {
    use musicwasp_core::smart::{Field, Op, SortBy};

    let folder = app
        .smart_dialog
        .as_ref()
        .and_then(|draft| draft.folder)
        .map(|id| app.playlist_folder_path(id));
    let Some(draft) = &mut app.smart_dialog else {
        return;
    };

    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    let mut remove_rule: Option<usize> = None;
    let editing = draft.editing.is_some();

    egui::Window::new(if editing {
        "Edit smart playlist"
    } else {
        "New smart playlist"
    })
    .open(&mut open)
    .collapsible(false)
    .resizable(true)
    .default_width(560.0)
    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    .show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr("ui.name"))
                    .size(12.0)
                    .color(th::text_dim()),
            );
            ui.add_sized([240.0, 22.0], egui::TextEdit::singleline(&mut draft.name));
        });
        if let Some(folder) = folder {
            ui.label(
                RichText::new(format!("Folder: {folder}"))
                    .size(10.5)
                    .color(th::text_dim()),
            );
        }

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
            if ui.button(tr("ui.add_rule")).clicked() {
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
        ui.label(
            RichText::new("Smart playlists re-query the library every time they open")
                .size(10.0)
                .color(th::text_dim()),
        );
        ui.add_space(4.0);
        let save_label = if editing {
            "Save changes"
        } else {
            tr("common.create")
        };
        let (create_clicked, cancel_clicked) = crate::dialog::save_cancel(ui, save_label, true);
        create |= create_clicked;
        cancel |= cancel_clicked;
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
