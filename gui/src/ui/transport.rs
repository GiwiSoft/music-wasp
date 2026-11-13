//! Transport bar, seek bar and now-playing view.

use super::*;

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
    if icons::transport(ui, icon, size.main, true, tr("transport.play_pause")).clicked() {
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

/// Seek bar with a fixed-size pill cursor. The slider still owns input,
/// keyboard support and accessibility; its handle is sized to match the fixed
/// cursor and then painted over, so the cursor cannot stretch with the layout.
pub(crate) fn seek_slider(ui: &mut egui::Ui, pos: &mut f64, max: f64) -> egui::Response {
    let height = ui
        .text_style_height(&egui::TextStyle::Body)
        .max(ui.spacing().interact_size.y);
    // Sliders use the accent for their filled trail; keep that scope local so
    // toggles in the same bar stay neutral.
    ui.visuals_mut().selection.bg_fill = th::accent();
    let radius = (height / 2.5).max(1.0);
    let aspect_ratio = (th::SEEK_HANDLE_WIDTH / 2.0) / radius;
    let response = ui.add(
        egui::Slider::new(pos, 0.0..=max)
            .show_value(false)
            .trailing_fill(true)
            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio }),
    );

    let visuals = ui.style().interact(&response);
    let fraction = if max > 0.0 {
        (*pos / max).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let half = th::SEEK_HANDLE_WIDTH / 2.0;
    let x = response.rect.left() + half + fraction * (response.rect.width() - 2.0 * half);
    let center = egui::pos2(x, response.rect.center().y);
    ui.painter().rect(
        egui::Rect::from_center_size(
            center,
            egui::vec2(th::SEEK_HANDLE_WIDTH, th::SEEK_HANDLE_HEIGHT),
        ),
        egui::CornerRadius::same((th::SEEK_HANDLE_WIDTH / 2.0) as u8),
        visuals.bg_fill,
        visuals.fg_stroke,
        egui::StrokeKind::Inside,
    );
    response
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
    let stream = app.stream.info.clone();
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
                        egui::Label::new(th::semibold(title, th::TEXT_TITLE).color(th::text()))
                            .truncate(),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(sub)
                                .size(th::TEXT_SMALL)
                                .color(th::text_dim()),
                        )
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
                    ui.visuals_mut().selection.bg_fill = th::accent();
                    let volume =
                        ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                    ui.visuals_mut().selection.bg_fill = th::bg_selected();
                    volume.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.volume"))
                    });
                    ui.label(icons::text(icons::SPEAKER_HIGH, 14.0).color(th::text_dim()))
                        .on_hover_text(tr("transport.volume"));
                    if icons::toggle(
                        ui,
                        app.settings.lyrics_open,
                        icons::QUOTES,
                        tr("lyrics.toggle"),
                    )
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
                                .size(th::TEXT_CAPTION)
                                .color(th::text_dim()),
                        ),
                    );
                    match stream.kind {
                        StreamKind::Radio => {
                            ui.label(
                                RichText::new(tr("transport.live"))
                                    .size(th::TEXT_CAPTION)
                                    .color(th::accent()),
                            );
                        }
                        StreamKind::Podcast => {
                            ui.label(
                                RichText::new(tr("transport.streaming").replace(
                                    "{duration}",
                                    &if duration > 0.0 {
                                        fmt_time(duration)
                                    } else {
                                        tr("transport.unknown_length").to_string()
                                    },
                                ))
                                .size(th::TEXT_CAPTION)
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
                                .size(th::TEXT_CAPTION)
                                .color(th::text_dim()),
                        ),
                    );
                    let mut pos = app.seek_drag.unwrap_or(snap.position);
                    let max = duration.max(1.0);
                    ui.spacing_mut().slider_width = (ui.available_width() - 100.0).max(100.0);
                    let resp = seek_slider(ui, &mut pos, max);
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
                                .size(th::TEXT_CAPTION)
                                .color(th::text_dim()),
                        ),
                    );
                });
            }

            if let Some(err) = &app.audio_error {
                ui.label(
                    RichText::new(
                        tr("transport.audio_unavailable").replace("{error}", &err.to_string()),
                    )
                    .size(th::TEXT_CAPTION)
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
        let visualizer_on = app.settings.visualizer.enabled;
        // Wide windows get the visualizer and a solid metadata panel side by
        // side; narrow ones keep the centred stack over a calm scrim.
        let split = visualizer_on && avail.x >= 880.0;
        let panel_width = (avail.x * 0.38).clamp(380.0, 460.0);

        if visualizer_on {
            let viz_width = if split {
                (avail.x - panel_width).max(320.0)
            } else {
                avail.x
            };
            let viz_size = egui::vec2(viz_width, avail.y);
            if let Some(texture) = app.render_visualizer(ui.ctx(), viz_size) {
                let rect = egui::Rect::from_min_size(ui.max_rect().min, viz_size);
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let scrim = if split {
                    30
                } else if th::skin().is_light() {
                    120
                } else {
                    205
                };
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::ZERO,
                    Color32::from_black_alpha(scrim),
                );
            }
        }

        let content_rect = if split {
            let panel = egui::Rect::from_min_size(
                egui::pos2(ui.max_rect().right() - panel_width, ui.max_rect().top()),
                egui::vec2(panel_width, avail.y),
            );
            ui.painter().rect_filled(
                panel,
                egui::CornerRadius::ZERO,
                th::alpha(th::bg_dark(), 246),
            );
            ui.painter().vline(
                panel.left(),
                panel.y_range(),
                egui::Stroke::new(1.0, th::hairline()),
            );
            panel.shrink2(egui::vec2(26.0, 22.0))
        } else {
            ui.max_rect().shrink2(egui::vec2(18.0, 14.0))
        };
        let width = content_rect.width();
        let height = content_rect.height();

        let mut body = |ui: &mut egui::Ui, width: f32, height: f32| {
            if let Some(stream) = app.stream.info.clone() {
                ui.vertical_centered(|ui| {
                    ui.add_space(if split { 8.0 } else { 30.0 });
                    let art = (height * 0.30).clamp(120.0, 280.0).min(width * 0.62);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(art, art), egui::Sense::hover());
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
                        egui::Label::new(
                            th::semibold(&stream.title, th::TEXT_DISPLAY).color(th::text()),
                        )
                        .truncate(),
                    );
                    ui.label(
                        RichText::new(&stream.subtitle)
                            .size(th::TEXT_TITLE)
                            .color(th::text_dim()),
                    );
                    ui.label(
                        RichText::new(match stream.kind {
                            StreamKind::Radio => tr("radio.subtitle"),
                            StreamKind::Podcast => tr("podcasts.subtitle"),
                        })
                        .size(th::TEXT_CAPTION)
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
                            RichText::new(format!("{position}  ·  {}", tr("transport.live")))
                                .size(th::TEXT_SMALL)
                                .color(th::accent()),
                        );
                    } else if stream.duration_secs > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{position} / {}",
                                fmt_time(stream.duration_secs as f64)
                            ))
                            .size(th::TEXT_SMALL)
                            .color(th::text_dim()),
                        );
                    } else {
                        ui.label(
                            RichText::new(position)
                                .size(th::TEXT_SMALL)
                                .color(th::text_dim()),
                        );
                    }
                });
                return;
            }
            let art = if split {
                (height * 0.30).clamp(150.0, 300.0).min(width * 0.72)
            } else {
                (height * 0.40).clamp(140.0, 330.0).min(width * 0.42)
            };

            ui.vertical_centered(|ui| {
                ui.add_space(if split { 6.0 } else { 10.0 });
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
                ui.add(
                    egui::Label::new(th::semibold(title, th::TEXT_DISPLAY).color(th::text()))
                        .truncate(),
                );

                ui.add_space(2.0);
                let subtitle = current_track
                    .as_ref()
                    .map(artist_album_line)
                    .unwrap_or_else(|| tr("transport.pick_track").to_string());
                ui.add(
                    egui::Label::new(
                        RichText::new(subtitle)
                            .size(th::TEXT_TITLE)
                            .color(th::text_dim()),
                    )
                    .truncate(),
                );
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
                let seek_row = if split {
                    width.min(360.0)
                } else {
                    (width * 0.46).clamp(360.0, 620.0)
                };
                ui.allocate_ui(egui::vec2(seek_row, 24.0), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(fmt_time(app.seek_drag.unwrap_or(snap.position)))
                                .monospace()
                                .size(th::TEXT_SMALL)
                                .color(th::text_dim()),
                        );
                        let mut pos = app.seek_drag.unwrap_or(snap.position);
                        let max = duration.max(1.0);
                        ui.spacing_mut().slider_width = (seek_row - 100.0).max(160.0);
                        let resp = seek_slider(ui, &mut pos, max);
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
                                .size(th::TEXT_SMALL)
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
                        if icons::toggle(
                            ui,
                            app.settings.lyrics_open,
                            icons::QUOTES,
                            tr("lyrics.toggle"),
                        )
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
                        ui.visuals_mut().selection.bg_fill = th::accent();
                        let volume =
                            ui.add(egui::Slider::new(&mut app.volume, 0.0..=1.0).show_value(false));
                        ui.visuals_mut().selection.bg_fill = th::bg_selected();
                        volume.widget_info(|| {
                            WidgetInfo::labeled(WidgetType::Slider, true, tr("transport.volume"))
                        });
                    });
                });

                ui.add_space(if split { 18.0 } else { 26.0 });
                match &upcoming {
                    Some(track) => {
                        ui.label(
                            th::semibold(
                                format!("{} {}", icons::QUEUE.as_str(), tr("queue.up_next_upper")),
                                th::TEXT_CAPTION,
                            )
                            .color(th::text_dim()),
                        );
                        ui.add_space(4.0);
                        ui.label(th::semibold(&track.title, th::TEXT_TITLE).color(th::text()));
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{}  ·  {}", track.artist, track.album))
                                    .size(th::TEXT_SMALL)
                                    .color(th::text_dim()),
                            )
                            .truncate(),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                icons::CHECK.as_str(),
                                tr("transport.end_of_queue")
                            ))
                            .size(th::TEXT_SMALL)
                            .color(th::text_dim()),
                        );
                    }
                }

                if let Some(err) = &app.audio_error {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            icons::WARNING.as_str(),
                            tr("transport.audio_unavailable").replace("{error}", &err.to_string())
                        ))
                        .size(th::TEXT_CAPTION)
                        .color(Color32::from_rgb(220, 110, 110)),
                    );
                }
            });
        };

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| body(ui, width, height),
        );
    });

    if let Some(p) = &app.player {
        if (app.volume - app.volume_sent).abs() > f32::EPSILON {
            p.set_volume(app.volume);
            app.volume_sent = app.volume;
        }
    }
}
