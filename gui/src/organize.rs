//! File organizer window: pattern, collision preview, apply and undo.

use crate::app::App;
use crate::icons;
use crate::theme as th;
use egui::{RichText, WidgetInfo, WidgetType};
use musicwasp_core::organize::{MoveStatus, OrganizePlan};
use std::path::Path;

fn hint(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .size(th::TEXT_CAPTION)
        .color(th::text_dim())
}

fn field_label(text: &str) -> RichText {
    RichText::new(text)
        .size(th::TEXT_SMALL)
        .color(th::text_dim())
}

fn status_color(status: MoveStatus) -> egui::Color32 {
    match status {
        MoveStatus::Ready => th::accent(),
        MoveStatus::Collision | MoveStatus::Missing | MoveStatus::Invalid => {
            egui::Color32::from_rgb(232, 120, 120)
        }
        MoveStatus::Unchanged => th::text_dim(),
    }
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.show_organize {
        return;
    }
    let selection = app.selected_in_view().len();
    let undo_count = app.move_undo.as_ref().map_or(0, Vec::len);
    let Some(state) = app.organize.as_mut() else {
        app.show_organize = false;
        return;
    };

    let mut open = true;
    let mut preview = false;
    let mut apply = false;
    let mut undo = false;

    egui::Window::new("Organize files from tags")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(880.0)
        .min_width(640.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(hint(
                "Move or rename files using their tags. Nothing changes until you apply the preview.",
            ));

            egui::Grid::new("organize_fields")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(field_label("Music folder"));
                    ui.add(
                        egui::TextEdit::singleline(&mut state.root)
                            .desired_width(560.0)
                            .hint_text("/home/me/Music"),
                    );
                    ui.end_row();
                    ui.label(field_label("Pattern"));
                    let pattern = ui.add(
                        egui::TextEdit::singleline(&mut state.pattern)
                            .desired_width(560.0)
                            .hint_text(musicwasp_core::organize::DEFAULT_PATTERN),
                    );
                    pattern.on_hover_text(
                        "Tokens: {artist} {album} {album_artist} {title} {track} {disc} \
                         {year} {genre} {ext}. Add a width such as {track:02}.",
                    );
                    ui.end_row();
                });
            ui.label(hint(
                "Tokens: {artist} {album} {album_artist} {title} {track} {disc} {year} \
                 {genre} {ext}. Use {track:02} to zero-pad numbers.",
            ));

            ui.horizontal(|ui| {
                if selection == 0 {
                    state.selected_only = false;
                }
                ui.add_enabled_ui(selection > 0, |ui| {
                    ui.checkbox(
                        &mut state.selected_only,
                        format!("Selected tracks only ({selection})"),
                    );
                });
                let can_move = state.plan.as_ref().is_some_and(OrganizePlan::has_moves);
                if ui
                    .button(icons::with(icons::MAGNIFYING_GLASS, "Preview"))
                    .clicked()
                {
                    preview = true;
                }
                if ui
                    .add_enabled(
                        can_move,
                        egui::Button::new(icons::with(icons::ARROWS_CLOCKWISE, "Move files")),
                    )
                    .clicked()
                {
                    state.pending = true;
                }
                if ui
                    .add_enabled(
                        undo_count > 0,
                        egui::Button::new(icons::with(
                            icons::ARROW_COUNTER_CLOCKWISE,
                            &format!("Undo last moves ({undo_count})"),
                        )),
                    )
                    .clicked()
                {
                    undo = true;
                }
            });

            ui.separator();
            if state.pending {
                let count = state.plan.as_ref().map_or(0, |plan| plan.ready);
                ui.label(
                    RichText::new(format!("Move {count} file(s) now?"))
                        .size(th::TEXT_BODY)
                        .strong()
                        .color(th::accent()),
                );
                ui.label(hint(
                    "Files are renamed on disk and their library rows follow. Undo restores both.",
                ));
                ui.horizontal(|ui| {
                    if ui
                        .button(icons::with(icons::ARROWS_CLOCKWISE, "Move files"))
                        .clicked()
                    {
                        apply = true;
                        state.pending = false;
                    }
                    if ui.button("Cancel").clicked() {
                        state.pending = false;
                    }
                });
            } else if let Some(status) = &state.status {
                ui.label(RichText::new(status).size(th::TEXT_CAPTION).color(th::text_dim()));
            }

            if let Some(plan) = &state.plan {
                ui.add_space(4.0);
                let root = Path::new(state.root.trim());
                egui::ScrollArea::vertical()
                    .id_salt("organize_rows")
                    .max_height(380.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("organize_table")
                            .num_columns(3)
                            .striped(true)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Status").strong());
                                ui.label(RichText::new("Current").strong());
                                ui.label(RichText::new("Target").strong());
                                ui.end_row();
                                for row in &plan.rows {
                                    let colour = status_color(row.status);
                                    let status = ui.label(
                                        RichText::new(row.status.label()).size(th::TEXT_CAPTION).color(colour),
                                    );
                                    if !row.detail.is_empty() {
                                        status.on_hover_text(&row.detail);
                                    }
                                    path_cell(ui, &row.current_path, root);
                                    path_cell(ui, &row.target_path, root);
                                    ui.end_row();
                                }
                            });
                    });
                ui.label(hint(format!(
                    "{} ready · {} skipped · {} collision(s)",
                    plan.ready,
                    plan.skipped,
                    plan.rows
                        .iter()
                        .filter(|row| row.status == MoveStatus::Collision)
                        .count()
                )));
            }
        });

    app.show_organize = open;
    if preview {
        app.organize_preview();
    }
    if apply {
        app.organize_apply();
    }
    if undo {
        app.undo_file_moves();
    }
}

fn path_cell(ui: &mut egui::Ui, path: &Path, root: &Path) {
    let text = relative(path, root);
    let label = ui.add(
        egui::Label::new(
            RichText::new(&text)
                .size(th::TEXT_CAPTION)
                .color(th::text()),
        )
        .truncate()
        .sense(egui::Sense::hover()),
    );
    label.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    label.on_hover_text(path.display().to_string());
}
