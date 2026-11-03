//! Skin colour editor with live preview.

use crate::app::App;
use crate::theme::{self, PaletteField};
use egui::RichText;

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.show_skin_editor {
        return;
    }
    let Some(state) = app.skin_editor.as_mut() else {
        app.show_skin_editor = false;
        return;
    };

    let mut open = true;
    let mut changed = false;
    let mut save = false;
    let mut cancel = false;
    let mut reset = false;

    egui::Window::new("Skin colours")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!(
                    "Editing \"{}\". Changes preview immediately.",
                    state.skin.label()
                ))
                .size(10.5)
                .color(theme::text_dim()),
            );
            ui.add_space(6.0);
            egui::Grid::new("skin_palette")
                .num_columns(2)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    for field in PaletteField::ALL {
                        ui.label(RichText::new(field.label()).size(11.0));
                        let mut color = field.get(&state.palette);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            field.set(&mut state.palette, color);
                            changed = true;
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            if ui.small_button("Reset").clicked() {
                reset = true;
            }
            ui.add_space(4.0);
            let (save_clicked, cancel_clicked) =
                crate::dialog::save_cancel(ui, "Save to skins.json", true);
            save |= save_clicked;
            cancel |= cancel_clicked;
            if let Some(status) = &state.status {
                ui.label(RichText::new(status).size(10.5).color(theme::accent()));
            }
        });

    if !open && !save {
        cancel = true;
    }

    if reset {
        if let Some(source) = app.skin_editor.as_ref().map(|editor| editor.skin.palette()) {
            if let Some(editor) = &mut app.skin_editor {
                editor.palette = source;
            }
            changed = true;
        }
    }

    if changed {
        let palette = app.skin_editor.as_ref().map(|editor| editor.palette);
        if let Some(palette) = palette {
            app.preview_skin_palette(ctx, palette);
        }
    }
    if save {
        app.save_skin_editor(ctx);
    } else if cancel {
        app.cancel_skin_editor(ctx);
    } else {
        app.show_skin_editor = open;
    }
}
