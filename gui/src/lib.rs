//! MusicWasp GUI library.
//!
//! The binary is a thin `eframe` launcher; keeping the application in a library
//! lets the integration tests under `gui/tests` drive the real app.

pub mod app;
mod backup_dialog;
mod dialog;
mod equalizer;
mod hygiene;
pub mod i18n;
pub mod icons;
mod mini_player;
#[cfg(target_os = "linux")]
mod mpris;
mod organize;
pub mod settings;
mod skin_editor;
pub mod theme;
#[cfg(target_os = "linux")]
mod tray;
mod ui;
pub(crate) mod visualizer;
