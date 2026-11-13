//! Shared window action layout: close in the title bar, primary actions at
//! the bottom right.

use crate::i18n::tr;
use crate::theme as th;
use egui::{Response, RichText};

/// Off-white primary button used for Save/Create/Apply. Green stays reserved
/// for playback and the single active indicator.
pub fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    let button = egui::Button::new(RichText::new(label).strong().color(th::current().bg_dark))
        .fill(th::text())
        .min_size(egui::vec2(84.0, 28.0));
    if enabled {
        ui.add(button)
    } else {
        ui.add_enabled(false, button)
    }
}

fn action_width(ui: &egui::Ui, save: &str, cancel: &str) -> f32 {
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(
                text.to_string(),
                egui::FontId::proportional(13.0),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
    };
    (measure(save).max(measure(cancel)) + 30.0).max(84.0)
}

/// Right-aligned primary action plus Cancel, both the same size.
pub fn save_cancel(ui: &mut egui::Ui, save_label: &str, save_enabled: bool) -> (bool, bool) {
    let mut save = false;
    let mut cancel = false;
    let cancel_label = tr("common.cancel");
    let size = egui::vec2(action_width(ui, save_label, cancel_label), 28.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let save_button = egui::Button::new(
            RichText::new(save_label)
                .strong()
                .color(th::current().bg_dark),
        )
        .fill(th::text());
        let response = if save_enabled {
            ui.add_sized(size, save_button)
        } else {
            ui.add_enabled_ui(false, |ui| ui.add_sized(size, save_button))
                .inner
        };
        if response.clicked() {
            save = true;
        }
        if ui
            .add_sized(size, egui::Button::new(cancel_label))
            .clicked()
        {
            cancel = true;
        }
    });
    (save, cancel)
}

/// Right-aligned primary Close for windows that apply changes immediately.
pub fn close_row(ui: &mut egui::Ui) -> bool {
    let mut close = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if primary_button(ui, tr("common.close"), true).clicked() {
            close = true;
        }
    });
    close
}
