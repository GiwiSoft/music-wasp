//! Restore-library dialog: pick a backup and replace the current database.

use crate::app::App;
use crate::theme as th;
use egui::RichText;
use std::path::Path;

fn hint(text: impl Into<String>) -> RichText {
    RichText::new(text).size(10.5).color(th::text_dim())
}

fn size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(draft) = app.restore_dialog.as_mut() else {
        return;
    };

    let mut open = true;
    let mut ask = false;
    let mut restore = false;
    let mut cancel = false;
    let mut refresh = false;

    egui::Window::new("Restore library")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(hint(
                "Restoring replaces the current library database. Settings, skins and undo journals \
                 are kept. A safety backup of the current library is written first.",
            ));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Backups").size(10.0).strong());
                if ui.small_button("Refresh").clicked() {
                    refresh = true;
                }
            });
            if draft.backups.is_empty() {
                ui.label(hint("No backups yet. Use the Back up button in Settings."));
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("backup_list")
                    .max_height(120.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for backup in &draft.backups {
                            let name = backup
                                .path
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let label = format!("{name}  ({})", size(backup.bytes));
                            if ui
                                .selectable_label(draft.path == backup.path.to_string_lossy().as_ref(), label)
                                .clicked()
                            {
                                draft.path = backup.path.to_string_lossy().to_string();
                                draft.confirm = false;
                            }
                        }
                    });
            }

            ui.add_space(6.0);
            ui.label(RichText::new("Backup file").size(10.0).strong());
            ui.add(
                egui::TextEdit::singleline(&mut draft.path)
                    .desired_width(500.0)
                    .hint_text("/path/to/library.db"),
            );

            ui.add_space(6.0);
            if draft.confirm {
                ui.label(
                    RichText::new("Replace the current library with this backup?")
                        .size(11.5)
                        .strong()
                        .color(th::accent()),
                );
                let (restore_clicked, cancel_clicked) =
                    crate::dialog::save_cancel(ui, "Restore library", true);
                restore |= restore_clicked;
                cancel |= cancel_clicked;
            } else {
                let (restore_clicked, cancel_clicked) = crate::dialog::save_cancel(
                    ui,
                    "Restore…",
                    !draft.path.trim().is_empty(),
                );
                ask |= restore_clicked;
                cancel |= cancel_clicked;
            }

            if let Some(status) = &draft.status {
                ui.label(RichText::new(status).size(10.5).color(th::accent()));
            }
        });

    if !open && !restore {
        cancel = true;
    }
    if refresh {
        if let Some(draft) = &mut app.restore_dialog {
            draft.backups = musicwasp_core::backup::list().unwrap_or_default();
        }
    }
    if ask {
        if let Some(draft) = &mut app.restore_dialog {
            draft.confirm = true;
        }
    }
    if cancel {
        app.restore_dialog = None;
    }
    if restore {
        let path = app
            .restore_dialog
            .as_ref()
            .map(|draft| draft.path.trim().to_string())
            .unwrap_or_default();
        match app.restore_library(Path::new(&path)) {
            Ok(message) => {
                app.restore_dialog = None;
                app.message = message;
            }
            Err(error) => {
                if let Some(draft) = &mut app.restore_dialog {
                    draft.status = Some(error);
                }
            }
        }
    }
}
