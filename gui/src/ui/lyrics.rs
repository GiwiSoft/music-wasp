//! Lyrics panel for the current local track.

use super::*;

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
    let stream = app.stream.info.clone();

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
                    th::semibold(
                        format!("{} {}", icons::QUOTES.as_str(), tr("lyrics.title")),
                        10.0,
                    )
                    .color(th::text_dim()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::compact(ui, icons::X, tr("lyrics.hide")).clicked() {
                        close = true;
                    }
                    if icons::compact(ui, icons::ARROWS_CLOCKWISE, tr("lyrics.reload")).clicked() {
                        reload = true;
                    }
                    if stream.is_none()
                        && app.lyrics.is_some()
                        && icons::toggle(
                            ui,
                            app.lyrics_autoscroll,
                            icons::ARROW_DOWN,
                            tr("lyrics.autoscroll"),
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
                        RichText::new(tr("lyrics.local_only"))
                            .size(th::TEXT_SMALL)
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
                    ui.label(th::semibold(tr("lyrics.none"), 13.0).color(th::text()));
                    ui.add_space(4.0);
                    let hint = current_track
                        .as_ref()
                        .map(|track| {
                            let path = std::path::Path::new(&track.path).with_extension("lrc");
                            tr("lyrics.add_sidecar").replace("{path}", &path.display().to_string())
                        })
                        .unwrap_or_else(|| tr("lyrics.start_track").to_string());
                    ui.label(
                        RichText::new(hint)
                            .size(th::TEXT_CAPTION)
                            .color(th::text_dim()),
                    );
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
                            th::semibold(&line.text, th::TEXT_TITLE).color(th::accent())
                        } else {
                            RichText::new(&line.text)
                                .size(th::TEXT_BODY)
                                .color(if !synced {
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
                                .on_hover_text(
                                    tr("lyrics.seek_to")
                                        .replace("{time}", &fmt_time(line.time_ms as f64 / 1000.0)),
                                );
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
