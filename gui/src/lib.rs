//! MusicWasp GUI library.
//!
//! The binary is a thin `eframe` launcher; keeping the application in a library
//! lets the integration tests under `gui/tests` drive the real app.

pub mod app;
mod equalizer;
mod hygiene;
pub mod i18n;
pub mod icons;
#[cfg(target_os = "linux")]
mod mpris;
mod organize;
pub mod settings;
mod skin_editor;
pub mod theme;
mod ui;
