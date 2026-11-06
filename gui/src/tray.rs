//! Optional Linux tray icon (StatusNotifierItem) with transport controls.

use std::sync::{Arc, Mutex, OnceLock};

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};

#[derive(Debug, Clone, Copy)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Show,
    ToggleMini,
    Quit,
}

struct TrayState {
    commands: Arc<Mutex<Vec<Command>>>,
    repaint: egui::Context,
    title: String,
    artist: String,
    playing: bool,
    mini: bool,
}

impl TrayState {
    fn send(&self, command: Command) {
        self.commands
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(command);
        self.repaint.request_repaint();
    }

    fn now_playing(&self) -> String {
        if self.title.is_empty() {
            "Nothing playing".to_string()
        } else if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artist, self.title)
        }
    }
}

impl ksni::Tray for TrayState {
    fn id(&self) -> String {
        "musicwasp".to_string()
    }

    fn title(&self) -> String {
        "MusicWasp".to_string()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon()]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "MusicWasp".to_string(),
            description: self.now_playing(),
            icon_pixmap: vec![icon()],
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Command::Show);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Command::PlayPause);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.now_playing(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: if self.playing { "Pause" } else { "Play" }.to_string(),
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::PlayPause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".to_string(),
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::Next)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".to_string(),
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::Previous)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Show MusicWasp".to_string(),
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::Show)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Mini player".to_string(),
                checked: self.mini,
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::ToggleMini)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut TrayState| tray.send(Command::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Tray created lazily when the setting is enabled.
pub struct TrayController {
    handle: ksni::blocking::Handle<TrayState>,
    commands: Arc<Mutex<Vec<Command>>>,
    last: Mutex<(String, String, bool, bool)>,
}

impl Drop for TrayController {
    fn drop(&mut self) {
        // Dropping the handle alone is not enough: the ksni service thread
        // keeps the StatusNotifierItem registered, so disabling or re-enabling
        // the tray would leave a stale icon behind. Its command channel is no
        // longer drained by the app, which made "Quit" in that stale menu do
        // nothing. Unregister explicitly and wait for the service to finish.
        self.handle.shutdown().wait();
    }
}

impl TrayController {
    pub fn spawn(ctx: &egui::Context) -> Result<Self, String> {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let state = TrayState {
            commands: Arc::clone(&commands),
            repaint: ctx.clone(),
            title: String::new(),
            artist: String::new(),
            playing: false,
            mini: false,
        };
        let handle = state.spawn().map_err(|error| error.to_string())?;
        Ok(Self {
            handle,
            commands,
            last: Mutex::new((String::new(), String::new(), false, false)),
        })
    }

    /// Push the current playback state; unchanged states skip the D-Bus call.
    pub fn update(&self, title: &str, artist: &str, playing: bool, mini: bool) {
        let next = (title.to_string(), artist.to_string(), playing, mini);
        {
            let mut last = self.last.lock().unwrap_or_else(|error| error.into_inner());
            if *last == next {
                return;
            }
            *last = next.clone();
        }
        self.handle.update(|tray| {
            tray.title = next.0;
            tray.artist = next.1;
            tray.playing = next.2;
            tray.mini = next.3;
        });
    }

    pub fn drain(&self) -> Vec<Command> {
        let mut commands = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *commands)
    }
}

fn icon() -> ksni::Icon {
    static ICON: OnceLock<ksni::Icon> = OnceLock::new();
    ICON.get_or_init(|| {
        let image = image::load_from_memory(include_bytes!("../../assets/MusicWasp-dark.png"))
            .expect("embedded logo")
            .to_rgba8();
        let image = image::imageops::resize(&image, 32, 32, image::imageops::FilterType::Lanczos3);
        let mut data = Vec::with_capacity(32 * 32 * 4);
        for pixel in image.pixels() {
            // StatusNotifierItem expects ARGB32 in network byte order.
            data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
        }
        ksni::Icon {
            width: 32,
            height: 32,
            data,
        }
    })
    .clone()
}
