//! Internet radio station view.

use super::*;

/// Internet radio station list.
pub fn radio_view(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .button(icons::with(icons::ARROW_LEFT, tr("library.title")))
                .clicked()
            {
                app.view = View::Library;
                app.mark_browser_dirty();
            }
            ui.label(th::semibold(tr("radio.title"), 18.0).color(th::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(icons::with(icons::PLUS, tr("radio.add_station")))
                    .clicked()
                {
                    app.radio.show_dialog = true;
                }
                let grid = app.radio.grid;
                if icons::compact(
                    ui,
                    if grid {
                        icons::LIST
                    } else {
                        icons::SQUARES_FOUR
                    },
                    if grid {
                        tr("ui.list_view")
                    } else {
                        tr("ui.grid_view")
                    },
                )
                .clicked()
                {
                    app.radio.grid = !grid;
                }
            });
        });
        if let Some(status) = &app.radio.status {
            ui.label(
                RichText::new(status.as_str())
                    .size(th::TEXT_CAPTION)
                    .color(th::text_dim()),
            );
        }
        ui.separator();
        if app.radio.stations.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(tr("radio.empty"))
                        .size(th::TEXT_BODY)
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
        if app.radio.grid {
            egui::ScrollArea::vertical()
                .id_salt("radio_stations_grid")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(16.0, 16.0);
                        for index in 0..app.radio.stations.len() {
                            let station = &app.radio.stations[index];
                            let texture = app.remote_covers.texture(
                                &ctx,
                                &station.image_url,
                                musicwasp_core::netguard::Access::AnyHost,
                            );
                            let playing = app.stream.info.as_ref().is_some_and(|stream| {
                                stream.kind == StreamKind::Radio && stream.url == station.url
                            });
                            let id = station.id;
                            let name = station.name.clone();
                            let genre = station.genre.clone();
                            let initials = initials(&name);
                            ui.allocate_ui_with_layout(
                                egui::vec2(168.0, 220.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    let cover = cover_art(
                                        ui,
                                        texture.as_ref(),
                                        156.0,
                                        &initials,
                                        icons::BROADCAST,
                                    );
                                    if cover.clicked() {
                                        station_action = Some(StationAction::Play(index));
                                    }
                                    let cover = cover.on_hover_text(tr("radio.play_station"));
                                    cover.widget_info(|| {
                                        WidgetInfo::labeled(WidgetType::Button, true, &name)
                                    });
                                    if playing {
                                        ui.painter().circle_filled(
                                            cover.rect.right_bottom() - egui::vec2(14.0, 14.0),
                                            4.0,
                                            th::accent(),
                                        );
                                    }
                                    cover.context_menu(|ui| {
                                        if ui.button(tr("radio.delete_station")).clicked() {
                                            station_action = Some(StationAction::Delete(id));
                                            ui.close();
                                        }
                                    });
                                    ui.add_space(6.0);
                                    ui.add(
                                        egui::Label::new(
                                            th::semibold(&name, th::TEXT_BODY).color(th::text()),
                                        )
                                        .truncate(),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(genre)
                                                .size(th::TEXT_CAPTION)
                                                .color(th::text_dim()),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if icons::compact(
                                                    ui,
                                                    icons::TRASH,
                                                    tr("radio.delete_station"),
                                                )
                                                .clicked()
                                                {
                                                    station_action =
                                                        Some(StationAction::Delete(id));
                                                }
                                            },
                                        );
                                    });
                                },
                            );
                        }
                    });
                });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("radio_stations")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for index in 0..app.radio.stations.len() {
                        let station = &app.radio.stations[index];
                        let texture = app.remote_covers.texture(
                            &ctx,
                            &station.image_url,
                            musicwasp_core::netguard::Access::AnyHost,
                        );
                        let playing = app.stream.info.as_ref().is_some_and(|stream| {
                            stream.kind == StreamKind::Radio && stream.url == station.url
                        });
                        let id = station.id;
                        let name = station.name.clone();
                        let genre = station.genre.clone();
                        let url = station.url.clone();
                        let initials = initials(&name);
                        ui.horizontal(|ui| {
                            if icons::compact(
                                ui,
                                if playing { icons::PAUSE } else { icons::PLAY },
                                tr("radio.play_station"),
                            )
                            .clicked()
                            {
                                station_action = Some(StationAction::Play(index));
                            }
                            let cover =
                                cover_art(ui, texture.as_ref(), 48.0, &initials, icons::BROADCAST);
                            if cover.clicked() {
                                station_action = Some(StationAction::Play(index));
                            }
                            let cover = cover.on_hover_text(tr("radio.play_station"));
                            cover.context_menu(|ui| {
                                if ui.button(tr("radio.delete_station")).clicked() {
                                    station_action = Some(StationAction::Delete(id));
                                    ui.close();
                                }
                            });
                            ui.vertical(|ui| {
                                ui.label(th::semibold(&name, th::TEXT_BODY).color(th::text()));
                                ui.label(
                                    RichText::new(genre)
                                        .size(th::TEXT_CAPTION)
                                        .color(th::text_dim()),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icons::compact(ui, icons::TRASH, tr("radio.delete_station"))
                                        .clicked()
                                    {
                                        station_action = Some(StationAction::Delete(id));
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(url)
                                                .size(th::TEXT_CAPTION)
                                                .color(th::text_dim()),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                        });
                        ui.add_space(4.0);
                    }
                });
        }
        match station_action {
            Some(StationAction::Play(index)) => {
                if let Some(station) = app.radio.stations.get(index).cloned() {
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
    if !app.radio.show_dialog {
        return;
    }
    let mut open = true;
    let mut name = app.radio.name.clone();
    let mut url = app.radio.url.clone();
    let mut genre = app.radio.genre.clone();
    let mut image_url = app.radio.image_url.clone();
    let mut add = false;
    egui::Window::new(tr("radio.add_dialog_title"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new(tr("ui.name")).size(th::TEXT_CAPTION));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut name).hint_text(tr("radio.station_name_hint")),
            );
            ui.label(RichText::new(tr("radio.stream_url")).size(th::TEXT_CAPTION));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut url).hint_text("https://example.com/stream.mp3"),
            );
            ui.label(RichText::new(tr("radio.genre_optional")).size(th::TEXT_CAPTION));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut genre).hint_text(tr("radio.genre_hint")),
            );
            ui.label(RichText::new(tr("radio.cover_url_optional")).size(th::TEXT_CAPTION));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut image_url).hint_text(tr("radio.cover_url_hint")),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("radio.add_station")).clicked() {
                    add = true;
                }
                if ui.button(tr("common.cancel")).clicked() {
                    app.radio.show_dialog = false;
                }
            });
        });
    app.radio.name = name;
    app.radio.url = url;
    app.radio.genre = genre;
    app.radio.image_url = image_url;
    if add {
        let (name, url, genre, image_url) = (
            app.radio.name.clone(),
            app.radio.url.clone(),
            app.radio.genre.clone(),
            app.radio.image_url.clone(),
        );
        app.add_radio_station(&name, &url, &genre, &image_url);
    }
    if !open {
        app.radio.show_dialog = false;
    }
}
