//! Podcast subscription and episode views.

use super::*;

/// Podcast subscriptions and episodes.
pub fn podcasts_view(app: &mut App, ui: &mut egui::Ui) {
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
            ui.label(th::semibold(tr("podcasts.title"), 18.0).color(th::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(icons::with(icons::PLUS, tr("podcasts.add_feed")))
                    .clicked()
                {
                    app.podcast.show_dialog = true;
                }
                if app.podcast.selected_feed.is_some()
                    && ui
                        .add_enabled(
                            !app.podcast.loading,
                            egui::Button::new(tr("common.refresh")),
                        )
                        .clicked()
                {
                    app.refresh_selected_podcast();
                }
            });
        });
        if let Some(status) = &app.podcast.status {
            ui.label(
                RichText::new(status.as_str())
                    .size(th::TEXT_CAPTION)
                    .color(th::text_dim()),
            );
        }
        if app.podcast.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("podcasts.fetching_feed")).size(th::TEXT_SMALL));
            });
        }
        ui.separator();

        if app.podcast.selected_feed.is_none() {
            if app.podcast.feeds.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(tr("podcasts.empty"))
                            .size(th::TEXT_BODY)
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
                    for index in 0..app.podcast.feeds.len() {
                        let feed = &app.podcast.feeds[index];
                        ui.horizontal(|ui| {
                            let label = tr("podcasts.feed_summary")
                                .replace("{title}", &feed.title)
                                .replace("{count}", &feed.episode_count.to_string())
                                .replace("{unplayed}", &feed.unplayed_count.to_string());
                            let width = (ui.available_width() - 36.0).max(80.0);
                            let response = ui.add_sized(
                                [width, 28.0],
                                egui::Label::new(
                                    RichText::new(label).size(th::TEXT_BODY).color(th::text()),
                                )
                                .sense(egui::Sense::click())
                                .truncate(),
                            );
                            if response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                feed_action = Some(FeedAction::Select(feed.id));
                            }
                            if icons::compact(ui, icons::TRASH, tr("podcasts.delete_subscription"))
                                .clicked()
                            {
                                feed_action = Some(FeedAction::Delete(feed.id));
                            }
                        });
                        ui.label(
                            RichText::new(feed.url.as_str())
                                .size(th::TEXT_CAPTION)
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
            let feed_id = app.podcast.selected_feed.unwrap_or_default();
            let title = app
                .podcast
                .feeds
                .iter()
                .find(|feed| feed.id == feed_id)
                .map(|feed| feed.title.clone())
                .unwrap_or_else(|| tr("podcasts.feed_fallback").to_string());
            ui.horizontal(|ui| {
                if ui.button(tr("podcasts.all_feeds")).clicked() {
                    app.podcast.selected_feed = None;
                    app.podcast.episodes.clear();
                }
                ui.label(th::semibold(title, th::TEXT_TITLE).color(th::text()));
                ui.label(
                    RichText::new(
                        tr("podcasts.episode_count")
                            .replace("{count}", &app.podcast.episodes.len().to_string()),
                    )
                    .size(th::TEXT_CAPTION)
                    .color(th::text_dim()),
                );
            });
            ui.add_space(4.0);
            if app.podcast.episodes.is_empty() {
                ui.label(
                    RichText::new(tr("podcasts.no_episodes"))
                        .size(th::TEXT_SMALL)
                        .color(th::text_dim()),
                );
                return;
            }
            let mut play_episode = None;
            // Two single-line labels plus the separator. `show_rows` needs an
            // exact height, so descriptions are truncated instead of wrapping.
            let row_height = 46.0;
            egui::ScrollArea::vertical()
                .id_salt("podcast_episodes")
                .auto_shrink([false, false])
                .show_rows(
                    ui,
                    row_height,
                    app.podcast.episodes.len(),
                    |ui, row_range| {
                        for index in row_range {
                            let episode = &app.podcast.episodes[index];
                            let width = ui.available_width();
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, row_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.horizontal(|ui| {
                                        let playing =
                                            app.stream.info.as_ref().is_some_and(|stream| {
                                                stream.episode_id == Some(episode.id)
                                            });
                                        if icons::compact(
                                            ui,
                                            if playing { icons::PAUSE } else { icons::PLAY },
                                            tr("podcasts.play_episode"),
                                        )
                                        .clicked()
                                        {
                                            play_episode = Some(index);
                                        }
                                        ui.vertical(|ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    th::semibold(&episode.title, th::TEXT_BODY)
                                                        .color(th::text()),
                                                )
                                                .truncate(),
                                            );
                                            let date = if episode.published.is_empty() {
                                                String::new()
                                            } else {
                                                format!("{}  ·  ", episode.published)
                                            };
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!(
                                                        "{}{}",
                                                        date,
                                                        if episode.description.is_empty() {
                                                            String::new()
                                                        } else {
                                                            episode
                                                                .description
                                                                .chars()
                                                                .take(110)
                                                                .collect()
                                                        }
                                                    ))
                                                    .size(th::TEXT_CAPTION)
                                                    .color(th::text_dim()),
                                                )
                                                .truncate(),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if episode.played {
                                                    ui.label(
                                                        icons::text(icons::CHECK, 12.0)
                                                            .color(th::accent()),
                                                    );
                                                } else if episode.position_secs > 0 {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} / {}",
                                                            fmt_time(episode.position_secs as f64),
                                                            fmt_time(episode.duration_secs as f64)
                                                        ))
                                                        .size(th::TEXT_CAPTION)
                                                        .color(th::text_dim()),
                                                    );
                                                } else if episode.duration_secs > 0 {
                                                    ui.label(
                                                        RichText::new(fmt_time(
                                                            episode.duration_secs as f64,
                                                        ))
                                                        .size(th::TEXT_CAPTION)
                                                        .color(th::text_dim()),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    ui.separator();
                                },
                            );
                        }
                    },
                );
            if let Some(index) = play_episode {
                if let Some(episode) = app.podcast.episodes.get(index).cloned() {
                    app.play_podcast_episode(&episode);
                }
            }
        }
    });
    podcast_dialog(app, &ctx);
}

fn podcast_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.podcast.show_dialog {
        return;
    }
    let mut open = true;
    let mut url = app.podcast.url.clone();
    let mut add = false;
    egui::Window::new(tr("podcasts.add_dialog_title"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new(tr("podcasts.feed_url")).size(th::TEXT_CAPTION));
            ui.add_sized(
                [460.0, 22.0],
                egui::TextEdit::singleline(&mut url).hint_text("https://example.com/feed.xml"),
            );
            ui.label(
                RichText::new(tr("podcasts.feed_storage_hint"))
                    .size(th::TEXT_CAPTION)
                    .color(th::text_dim()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("podcasts.add_feed")).clicked() {
                    add = true;
                }
                if ui.button(tr("common.cancel")).clicked() {
                    app.podcast.show_dialog = false;
                }
            });
        });
    app.podcast.url = url;
    if add {
        app.add_podcast_feed(&app.podcast.url.clone());
    }
    if !open {
        app.podcast.show_dialog = false;
    }
}
