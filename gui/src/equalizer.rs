//! Equalizer window: enable, preamp, 10 bands, presets and a live curve.

use crate::app::App;
use crate::theme as th;
use egui::{Color32, RichText, Sense, Stroke, WidgetInfo, WidgetType};
use musicwasp_core::dsp::{self, Equalizer, BAND_FREQS, GAIN_RANGE_DB, PREAMP_RANGE_DB};

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.show_equalizer {
        return;
    }

    let mut open = true;
    egui::Window::new("Equalizer")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(590.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let eq = &mut app.settings.equalizer;

            ui.horizontal(|ui| {
                ui.checkbox(&mut eq.enabled, "Enabled")
                    .on_hover_text("Bypass or apply the filter chain");

                let current = eq.preset_name().unwrap_or("Custom");
                egui::ComboBox::from_id_salt("eq_preset")
                    .selected_text(RichText::new(current).size(12.0))
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for preset in dsp::PRESETS.iter() {
                            if ui
                                .selectable_label(current == preset.name, preset.name)
                                .clicked()
                            {
                                eq.apply_preset(preset);
                            }
                        }
                    });

                if ui
                    .button("Reset")
                    .on_hover_text("Flat response, 0 dB preamp")
                    .clicked()
                {
                    eq.gains_db = [0.0; BAND_FREQS.len()];
                    eq.preamp_db = 0.0;
                }

                ui.separator();
                ui.label(
                    RichText::new(dsp::describe(eq))
                        .size(11.0)
                        .color(th::accent()),
                );
            });

            ui.add_space(6.0);
            curve(ui, eq);

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = 135.0;

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("PREAMP")
                            .size(9.0)
                            .strong()
                            .color(th::text_dim()),
                    );
                    let preamp = ui.add_sized(
                        [34.0, 145.0],
                        egui::Slider::new(&mut eq.preamp_db, -PREAMP_RANGE_DB..=PREAMP_RANGE_DB)
                            .vertical()
                            .show_value(false),
                    );
                    preamp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Slider, true, "Preamp gain")
                    });
                    ui.label(
                        RichText::new(format!("{:+.1}", eq.preamp_db))
                            .monospace()
                            .size(10.0)
                            .color(th::text_dim()),
                    );
                    ui.label(RichText::new("dB").size(9.0).color(th::text_dim()));
                });

                ui.separator();

                for (band, freq) in BAND_FREQS.iter().enumerate() {
                    ui.vertical(|ui| {
                        let gain = &mut eq.gains_db[band];
                        let slider = ui.add_sized(
                            [32.0, 145.0],
                            egui::Slider::new(gain, -GAIN_RANGE_DB..=GAIN_RANGE_DB)
                                .vertical()
                                .show_value(false),
                        );
                        slider.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Slider,
                                true,
                                format!("{freq} Hz band gain"),
                            )
                        });
                        ui.label(
                            RichText::new(format!("{:+.1}", *gain))
                                .monospace()
                                .size(9.0)
                                .color(if gain.abs() > 0.05 {
                                    th::accent()
                                } else {
                                    th::text_dim()
                                }),
                        );
                        ui.label(
                            RichText::new(dsp::band_label(*freq))
                                .size(10.0)
                                .color(th::text_dim()),
                        );
                    });
                }
            });

            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Changes apply to the current track immediately and are saved automatically.",
                )
                .size(10.0)
                .color(th::text_dim()),
            );
        });

    if !open {
        app.show_equalizer = false;
    }
}

/// Draw a log-frequency magnitude response for the current settings.
fn curve(ui: &mut egui::Ui, eq: &Equalizer) {
    let width = ui.available_width().clamp(320.0, 550.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 120.0), Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 3.0, th::bg_row_alt());
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0_f32, th::text_dim()),
        egui::StrokeKind::Inside,
    );

    const DB_TOP: f32 = 18.0;
    const DB_BOTTOM: f32 = -18.0;
    let y_for = |db: f32| {
        let t = ((DB_TOP - db) / (DB_TOP - DB_BOTTOM)).clamp(0.0, 1.0);
        rect.top() + t * rect.height()
    };
    let x_for = |freq: f32| {
        let t = ((freq / 20.0).ln() / (20_000.0f32 / 20.0).ln()).clamp(0.0, 1.0);
        rect.left() + t * rect.width()
    };

    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = y_for(db);
        let stroke = if db == 0.0 {
            Stroke::new(1.0_f32, th::text_dim())
        } else {
            Stroke::new(1.0_f32, th::bg_selected())
        };
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
        painter.text(
            egui::pos2(rect.left() + 4.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{db:+.0}"),
            egui::FontId::monospace(8.0),
            th::text_dim(),
        );
    }
    for freq in [100.0, 1_000.0, 10_000.0] {
        let x = x_for(freq);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, th::bg_selected()),
        );
    }

    let mut points = Vec::with_capacity(rect.width() as usize);
    let steps = rect.width().max(1.0) as usize;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let freq = 20.0 * (20_000.0f32 / 20.0).powf(t);
        let db = eq.response_db(freq, 48_000.0).clamp(DB_BOTTOM, DB_TOP);
        let x = rect.left() + t * rect.width();
        points.push(egui::pos2(x, y_for(db)));
    }

    let colour = if eq.enabled {
        th::accent()
    } else {
        Color32::from_rgba_unmultiplied(
            th::text_dim().r(),
            th::text_dim().g(),
            th::text_dim().b(),
            150,
        )
    };
    painter.add(egui::Shape::line(points, Stroke::new(1.7_f32, colour)));

    if !eq.enabled {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "BYPASSED",
            egui::FontId::proportional(11.0),
            th::text_dim(),
        );
    }
}
