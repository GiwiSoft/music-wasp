//! Internet radio station view.

use super::*;

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
                    app.radio.show_dialog = true;
                }
            });
        });
        if let Some(status) = &app.radio.status {
            ui.label(
                RichText::new(status.as_str())
                    .size(10.5)
                    .color(th::text_dim()),
            );
        }
        ui.separator();
        if app.radio.stations.is_empty() {
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
                for index in 0..app.radio.stations.len() {
                    let station = &app.radio.stations[index];
                    ui.horizontal(|ui| {
                        let playing = app.stream.info.as_ref().is_some_and(|stream| {
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
                    app.radio.show_dialog = false;
                }
            });
        });
    app.radio.name = name;
    app.radio.url = url;
    app.radio.genre = genre;
    if add {
        let (name, url, genre) = (
            app.radio.name.clone(),
            app.radio.url.clone(),
            app.radio.genre.clone(),
        );
        app.add_radio_station(&name, &url, &genre);
    }
    if !open {
        app.radio.show_dialog = false;
    }
}
