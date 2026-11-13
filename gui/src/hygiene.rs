//! Library hygiene window: missing files, duplicates and rename preview.

use crate::app::{App, HygieneState};
use crate::i18n::tr;
use crate::icons;
use crate::theme as th;
use egui::RichText;
use musicwasp_core::{ReconcileCandidate, Track};
use std::collections::BTreeSet;

fn hint(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .size(th::TEXT_CAPTION)
        .color(th::text_dim())
}

fn track_label(track: &Track) -> String {
    let mut label = if track.title.is_empty() {
        "(untitled)".to_string()
    } else {
        track.title.clone()
    };
    if !track.artist.is_empty() {
        label.push_str("  -  ");
        label.push_str(&track.artist);
    }
    label
}

fn selection_box(ui: &mut egui::Ui, selected: &mut BTreeSet<i64>, id: i64) {
    let mut checked = selected.contains(&id);
    if ui.checkbox(&mut checked, "").changed() {
        if checked {
            selected.insert(id);
        } else {
            selected.remove(&id);
        }
    }
}

fn path_line(ui: &mut egui::Ui, prefix: &str, path: &str) {
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!("{prefix}  {path}"))
                    .size(th::TEXT_CAPTION)
                    .color(th::text_dim()),
            )
            .truncate(),
        )
        .on_hover_text(path);
    });
}

/// One selectable track row: metadata on the first line, path underneath.
fn track_row(ui: &mut egui::Ui, selected: &mut BTreeSet<i64>, track: &Track) {
    ui.horizontal(|ui| {
        selection_box(ui, selected, track.id);
        ui.add(
            egui::Label::new(
                RichText::new(track_label(track))
                    .size(th::TEXT_SMALL)
                    .color(th::text()),
            )
            .truncate(),
        );
    });
    path_line(ui, "", &track.path);
}

fn rename_row(ui: &mut egui::Ui, selected: &mut BTreeSet<i64>, candidate: &ReconcileCandidate) {
    ui.horizontal(|ui| {
        selection_box(ui, selected, candidate.track.id);
        ui.add(
            egui::Label::new(
                RichText::new(track_label(&candidate.track))
                    .size(th::TEXT_SMALL)
                    .color(th::text()),
            )
            .truncate(),
        );
        if candidate.replaces_id.is_some() {
            ui.label(
                RichText::new("merges watcher duplicate")
                    .size(th::TEXT_CAPTION)
                    .color(th::accent()),
            );
        }
    });
    path_line(ui, "from", &candidate.track.path);
    path_line(ui, "to", &candidate.new_path);
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        ui.label(
            RichText::new(format!("matches: {}", candidate.reasons.join(" · ")))
                .size(th::TEXT_CAPTION)
                .color(th::text_dim()),
        );
    });
}

/// Actions raised by the maintenance window and applied after the frame.
enum HygieneAct {
    Rescan,
    FindRenames,
    SelectMissing,
    SelectDuplicates,
    SelectRenames,
    Clear,
    ClearRenames,
    Ask(Vec<i64>),
    AskRenames(Vec<i64>),
    Confirm,
    ConfirmRenames,
    Cancel,
    CancelRenames,
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.show_hygiene {
        return;
    }
    let Some(state) = app.hygiene.as_mut() else {
        app.show_hygiene = false;
        return;
    };

    let mut act = None;
    let mut open = app.show_hygiene;
    egui::Window::new(tr("menu.library_hygiene"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .min_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            take_act(&mut act, hygiene_toolbar(ui, state));
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("hygiene_scroll")
                .max_height(420.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    take_act(&mut act, hygiene_missing(ui, state));
                    ui.add_space(4.0);
                    take_act(&mut act, hygiene_duplicates(ui, state));
                    ui.add_space(4.0);
                    take_act(&mut act, hygiene_renames(ui, state));
                });
            ui.separator();
            take_act(&mut act, hygiene_footer(ui, state));
            if let Some(status) = &state.status {
                ui.label(
                    RichText::new(status)
                        .size(th::TEXT_CAPTION)
                        .color(th::accent()),
                );
            }
        });

    app.show_hygiene = open;
    if let Some(act) = act {
        apply_hygiene_act(app, act);
    }
}

/// Keep the first action raised in this frame; later sections are still drawn.
fn take_act(slot: &mut Option<HygieneAct>, act: Option<HygieneAct>) {
    if slot.is_none() {
        *slot = act;
    }
}

fn hygiene_toolbar(ui: &mut egui::Ui, state: &HygieneState) -> Option<HygieneAct> {
    let mut act = None;
    let duplicate_tracks: usize = state.duplicate_groups.iter().map(Vec::len).sum();
    ui.horizontal(|ui| {
        if ui
            .button(icons::with(icons::ARROWS_CLOCKWISE, "Rescan"))
            .clicked()
        {
            act = Some(HygieneAct::Rescan);
        }
        ui.label(
            RichText::new(format!(
                "{} missing · {} duplicate group(s) · {} duplicate track(s)",
                state.missing.len(),
                state.duplicate_groups.len(),
                duplicate_tracks
            ))
            .size(th::TEXT_SMALL)
            .color(th::text_dim()),
        );
    });
    ui.label(hint(
        "Removing entries never deletes audio files from disk.",
    ));
    act
}

fn hygiene_missing(ui: &mut egui::Ui, state: &mut HygieneState) -> Option<HygieneAct> {
    let mut act = None;
    egui::CollapsingHeader::new(format!("Missing files ({})", state.missing.len()))
        .default_open(true)
        .show(ui, |ui| {
            if state.missing.is_empty() {
                ui.label(hint("Every indexed file still exists."));
                return;
            }
            let rename_ids = rename_track_ids(state);
            ui.horizontal(|ui| {
                if ui.small_button("Select all").clicked() {
                    act = Some(HygieneAct::SelectMissing);
                }
                if ui.small_button("Prune all").clicked() {
                    // Rows with a rename match are left for the preview;
                    // pruning them here would lose their history.
                    let ids: Vec<i64> = state
                        .missing
                        .iter()
                        .filter(|track| !rename_ids.contains(&track.id))
                        .map(|track| track.id)
                        .collect();
                    if !ids.is_empty() {
                        act = Some(HygieneAct::Ask(ids));
                    }
                }
            });
            for track in &state.missing {
                track_row(ui, &mut state.selected, track);
            }
        });
    act
}

fn hygiene_duplicates(ui: &mut egui::Ui, state: &mut HygieneState) -> Option<HygieneAct> {
    let mut act = None;
    egui::CollapsingHeader::new(format!(
        "Duplicates ({} groups)",
        state.duplicate_groups.len()
    ))
    .default_open(true)
    .show(ui, |ui| {
        if state.duplicate_groups.is_empty() {
            ui.label(hint("No tracks share the same artist, title and album."));
            return;
        }
        if ui
            .small_button("Select all but the first in each group")
            .clicked()
        {
            act = Some(HygieneAct::SelectDuplicates);
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
                RichText::new(heading)
                    .size(th::TEXT_SMALL)
                    .strong()
                    .color(th::text()),
            );
            for track in group {
                track_row(ui, &mut state.selected, track);
            }
        }
    });
    act
}

fn hygiene_renames(ui: &mut egui::Ui, state: &mut HygieneState) -> Option<HygieneAct> {
    let mut act = None;
    let candidates = state
        .reconcile
        .as_ref()
        .map_or(0, |plan| plan.candidates.len());
    egui::CollapsingHeader::new(format!("Possible renames ({candidates})"))
        .default_open(true)
        .show(ui, |ui| {
            let Some(plan) = &state.reconcile else {
                ui.label(hint(
                    "Find rows whose files were renamed outside MusicWasp. \
                     Matches use tags, size, duration and mtime.",
                ));
                if ui
                    .button(icons::with(icons::MAGNIFYING_GLASS, "Find renames"))
                    .clicked()
                {
                    act = Some(HygieneAct::FindRenames);
                }
                return;
            };
            ui.label(
                RichText::new(format!(
                    "{} file(s) scanned · nothing changes until you apply",
                    plan.scanned_files
                ))
                .size(th::TEXT_CAPTION)
                .color(th::text_dim()),
            );
            if plan.candidates.is_empty() {
                ui.label(hint(
                    "No vanished row matches a file on disk. \
                     Prune the missing entries if the files are gone.",
                ));
                return;
            }
            ui.horizontal(|ui| {
                if ui.small_button("Select all").clicked() {
                    act = Some(HygieneAct::SelectRenames);
                }
                if ui.small_button("Clear").clicked() {
                    act = Some(HygieneAct::ClearRenames);
                }
            });
            for candidate in &plan.candidates {
                ui.add_space(6.0);
                rename_row(ui, &mut state.rename_selected, candidate);
            }
        });
    act
}

fn hygiene_footer(ui: &mut egui::Ui, state: &HygieneState) -> Option<HygieneAct> {
    if let Some(ids) = &state.pending_renames {
        return confirm_bar(
            ui,
            format!("Relocate {} track(s) to their new paths?", ids.len()),
            "Ratings, playlists and play history stay attached. Audio files are not modified.",
            icons::with(icons::ARROWS_CLOCKWISE, "Apply renames"),
            HygieneAct::ConfirmRenames,
            HygieneAct::CancelRenames,
        );
    }
    if let Some(ids) = &state.pending {
        return confirm_bar(
            ui,
            format!("Remove {} track(s) from the library?", ids.len()),
            "The audio files stay on disk and can be re-imported later.",
            icons::with(icons::TRASH, "Remove from library"),
            HygieneAct::Confirm,
            HygieneAct::Cancel,
        );
    }
    hygiene_selection_bar(ui, state)
}

fn confirm_bar(
    ui: &mut egui::Ui,
    title: String,
    note: &str,
    action: String,
    confirm: HygieneAct,
    cancel: HygieneAct,
) -> Option<HygieneAct> {
    let mut act = None;
    ui.label(
        RichText::new(title)
            .size(th::TEXT_BODY)
            .strong()
            .color(th::text()),
    );
    ui.label(hint(note));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if crate::dialog::primary_button(ui, &action, true).clicked() {
            act = Some(confirm);
        }
        if ui.button(tr("common.cancel")).clicked() {
            act = Some(cancel);
        }
    });
    act
}

fn hygiene_selection_bar(ui: &mut egui::Ui, state: &HygieneState) -> Option<HygieneAct> {
    let mut act = None;
    let rename_candidates = state
        .reconcile
        .as_ref()
        .map_or(0, |plan| plan.candidates.len());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} selected", state.selected.len()))
                .size(th::TEXT_SMALL)
                .color(th::text_dim()),
        );
        let remove_label = format!("Remove {} from library", state.selected.len());
        if ui
            .add_enabled(
                !state.selected.is_empty(),
                egui::Button::new(icons::with(icons::TRASH, &remove_label)),
            )
            .on_hover_text("Deletes library entries only; files stay on disk")
            .clicked()
        {
            act = Some(HygieneAct::Ask(state.selected.iter().copied().collect()));
        }
        if ui.small_button("Clear selection").clicked() && !state.selected.is_empty() {
            act = Some(HygieneAct::Clear);
        }
        if rename_candidates > 0 {
            let rename_label = format!("Apply {} rename(s)", state.rename_selected.len());
            if ui
                .add_enabled(
                    !state.rename_selected.is_empty(),
                    egui::Button::new(icons::with(icons::ARROWS_CLOCKWISE, &rename_label)),
                )
                .on_hover_text("Updates the indexed paths only; audio files are untouched")
                .clicked()
            {
                act = Some(HygieneAct::AskRenames(
                    state.rename_selected.iter().copied().collect(),
                ));
            }
        }
    });
    act
}

fn rename_track_ids(state: &HygieneState) -> BTreeSet<i64> {
    state
        .reconcile
        .as_ref()
        .map(|plan| plan.candidates.iter().map(|c| c.track.id).collect())
        .unwrap_or_default()
}

fn apply_hygiene_act(app: &mut App, act: HygieneAct) {
    match act {
        HygieneAct::Rescan => app.scan_hygiene(),
        HygieneAct::FindRenames => app.reconcile_hygiene(),
        HygieneAct::SelectMissing => {
            if let Some(state) = &mut app.hygiene {
                let rename_ids = rename_track_ids(state);
                state.selected = state
                    .missing
                    .iter()
                    .filter(|track| !rename_ids.contains(&track.id))
                    .map(|track| track.id)
                    .collect();
            }
        }
        HygieneAct::SelectDuplicates => {
            if let Some(state) = &mut app.hygiene {
                for group in &state.duplicate_groups {
                    state
                        .selected
                        .extend(group.iter().skip(1).map(|track| track.id));
                }
            }
        }
        HygieneAct::SelectRenames => {
            if let Some(state) = &mut app.hygiene {
                state.rename_selected = rename_track_ids(state);
            }
        }
        HygieneAct::Clear => {
            if let Some(state) = &mut app.hygiene {
                state.selected.clear();
            }
        }
        HygieneAct::ClearRenames => {
            if let Some(state) = &mut app.hygiene {
                state.rename_selected.clear();
            }
        }
        HygieneAct::Ask(ids) => {
            if let Some(state) = &mut app.hygiene {
                state.pending = Some(ids);
            }
        }
        HygieneAct::AskRenames(ids) => {
            if let Some(state) = &mut app.hygiene {
                state.pending_renames = Some(ids);
            }
        }
        HygieneAct::Confirm => {
            let ids = app
                .hygiene
                .as_mut()
                .and_then(|state| state.pending.take())
                .unwrap_or_default();
            app.remove_from_library(&ids);
        }
        HygieneAct::ConfirmRenames => {
            let ids = app
                .hygiene
                .as_mut()
                .and_then(|state| state.pending_renames.take())
                .unwrap_or_default();
            app.apply_hygiene_renames(&ids);
        }
        HygieneAct::Cancel => {
            if let Some(state) = &mut app.hygiene {
                state.pending = None;
            }
        }
        HygieneAct::CancelRenames => {
            if let Some(state) = &mut app.hygiene {
                state.pending_renames = None;
            }
        }
    }
}
