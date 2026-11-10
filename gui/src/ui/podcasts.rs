//! Podcast subscription and episode views.

use super::*;

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
            // Two single-line labels plus the separator. `show_rows` needs an
            // exact height, so descriptions are truncated instead of wrapping.
            let row_height = 46.0;
            egui::ScrollArea::vertical()
                .id_salt("podcast_episodes")
                .auto_shrink([false, false])
                .show_rows(
                    ui,
                    row_height,
                    app.podcast_episodes.len(),
                    |ui, row_range| {
                        for index in row_range {
                            let episode = &app.podcast_episodes[index];
                            let width = ui.available_width();
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, row_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.horizontal(|ui| {
                                        let playing = app.stream.as_ref().is_some_and(|stream| {
                                            stream.episode_id == Some(episode.id)
                                        });
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
                                                    th::semibold(&episode.title, 12.5)
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
                                                    .size(10.0)
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
                                                        .size(10.0)
                                                        .color(th::text_dim()),
                                                    );
                                                } else if episode.duration_secs > 0 {
                                                    ui.label(
                                                        RichText::new(fmt_time(
                                                            episode.duration_secs as f64,
                                                        ))
                                                        .size(10.0)
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
