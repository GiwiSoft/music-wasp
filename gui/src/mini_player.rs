//! Compact always-on-top player window.
//!
//! The window runs in its own egui viewport. It cannot borrow `App`, so the
//! main thread publishes a snapshot through [`MiniBridge`] and the window sends
//! transport commands back through the same bridge.

use crate::icons;
use crate::theme as th;
use egui::RichText;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
pub enum MiniCommand {
    PlayPause,
    Next,
    Previous,
    Seek(f64),
    ShowMain,
    Close,
}

#[derive(Default)]
pub struct MiniState {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub commands: Vec<MiniCommand>,
    pub close_requested: bool,
}

/// Shared snapshot and command queue between the main window and the mini one.
pub struct MiniBridge {
    state: Arc<Mutex<MiniState>>,
}

impl MiniBridge {
    pub fn viewport_id() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("musicwasp-mini")
    }

    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MiniState::default())),
        }
    }

    pub fn update(&self, snapshot: MiniSnapshot) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.title = snapshot.title;
        state.artist = snapshot.artist;
        state.album = snapshot.album;
        state.playing = snapshot.playing;
        state.position = snapshot.position;
        state.duration = snapshot.duration;
    }

    pub fn take_commands(&self) -> Vec<MiniCommand> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut state.commands)
    }

    pub fn close_requested(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.close_requested
    }

    pub fn shared(&self) -> Arc<Mutex<MiniState>> {
        Arc::clone(&self.state)
    }
}

pub struct MiniSnapshot {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
}

pub fn builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title("MusicWasp mini")
        .with_app_id("musicwasp-mini")
        .with_inner_size([330.0, 142.0])
        .with_min_inner_size([300.0, 132.0])
        .with_resizable(false)
        .with_always_on_top()
}

/// Draw the contents of the mini viewport.
pub fn ui(
    ui: &mut egui::Ui,
    state: &Arc<Mutex<MiniState>>,
    root: &egui::Context,
    skin: crate::theme::Skin,
) {
    crate::theme::apply(ui.ctx(), skin);

    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let mut commands = Vec::new();
    ui.visuals_mut().selection.bg_fill = th::accent();

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    th::semibold(
                        if state.title.is_empty() {
                            "Nothing playing"
                        } else {
                            state.title.as_str()
                        },
                        13.0,
                    )
                    .color(th::text()),
                )
                .truncate(),
            );
        });
        let mut subtitle = state.artist.clone();
        if !state.album.is_empty() {
            if !subtitle.is_empty() {
                subtitle.push_str(" · ");
            }
            subtitle.push_str(&state.album);
        }
        ui.add(
            egui::Label::new(RichText::new(subtitle).size(10.5).color(th::text_dim())).truncate(),
        );

        ui.add_space(4.0);
        let mut position = state.position;
        let slider = crate::ui::seek_slider(ui, &mut position, state.duration.max(0.0));
        if slider.changed() {
            commands.push(MiniCommand::Seek(position));
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    crate::app::fmt_time(state.position),
                    crate::app::fmt_time(state.duration)
                ))
                .size(9.5)
                .color(th::text_dim()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::X.as_str()).clicked() {
                    commands.push(MiniCommand::Close);
                }
                if ui
                    .small_button(icons::text(icons::MONITOR_PLAY, 13.0))
                    .on_hover_text("Show MusicWasp")
                    .clicked()
                {
                    commands.push(MiniCommand::ShowMain);
                }
                ui.add_space(6.0);
                if ui
                    .button(icons::text(icons::SKIP_FORWARD, 13.0))
                    .on_hover_text("Next")
                    .clicked()
                {
                    commands.push(MiniCommand::Next);
                }
                if ui
                    .button(icons::text(
                        if state.playing {
                            icons::PAUSE
                        } else {
                            icons::PLAY
                        },
                        13.0,
                    ))
                    .on_hover_text(if state.playing { "Pause" } else { "Play" })
                    .clicked()
                {
                    commands.push(MiniCommand::PlayPause);
                }
                if ui
                    .button(icons::text(icons::SKIP_BACK, 13.0))
                    .on_hover_text("Previous")
                    .clicked()
                {
                    commands.push(MiniCommand::Previous);
                }
            });
        });
    });

    if ui.input(|input| input.viewport().close_requested()) {
        state.close_requested = true;
        commands.push(MiniCommand::Close);
    }
    if !commands.is_empty() {
        state.commands.extend(commands);
        root.request_repaint();
    }
    if state.playing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
}
