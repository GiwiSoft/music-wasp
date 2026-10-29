//! Linux desktop integration: an MPRIS2 service so GNOME/KDE, lock screens,
//! `playerctl` and Bluetooth headset buttons can control MusicWasp.
//!
//! The MPRIS object lives on its own thread because `mpris-server` is built on
//! `Rc` and must stay on one async executor. The UI talks to it through two
//! channels: a `watch` channel for the latest playback snapshot, and a normal
//! channel for commands coming from the desktop.

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MprisStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoopMode {
    #[default]
    None,
    Track,
    Playlist,
}

/// Everything MPRIS clients need to display and control the player.
#[derive(Debug, Clone, Default)]
pub struct MprisSnapshot {
    pub status: MprisStatus,
    pub track_id: Option<i64>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    pub position_secs: f64,
    pub volume: f32,
    pub can_next: bool,
    pub can_previous: bool,
    pub shuffle: bool,
    pub loop_mode: LoopMode,
}

#[derive(Debug, Clone, Copy)]
pub enum MprisCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek(f64),
    SetPosition(f64),
    SetVolume(f32),
    SetShuffle(bool),
    SetLoop(LoopMode),
    Raise,
    Quit,
}

/// Handle kept by the UI. Dropping it closes the update channel, which ends the
/// service thread and releases the D-Bus name.
pub struct MprisService {
    commands: Receiver<MprisCommand>,
    updates: tokio::sync::watch::Sender<MprisSnapshot>,
}

#[derive(Clone)]
struct CommandSender {
    commands: Sender<MprisCommand>,
    ctx: egui::Context,
}

impl CommandSender {
    fn send(&self, command: MprisCommand) {
        let _ = self.commands.send(command);
        // Wake the UI so it polls the command even when the window is idle.
        self.ctx.request_repaint();
    }
}

impl MprisService {
    pub fn start(ctx: egui::Context) -> Option<Self> {
        let (command_tx, commands) = mpsc::channel();
        let (updates, update_rx) = tokio::sync::watch::channel(MprisSnapshot::default());
        let sender = CommandSender {
            commands: command_tx,
            ctx,
        };

        std::thread::Builder::new()
            .name("mpris".to_string())
            .spawn(move || run(sender, update_rx))
            .ok()?;

        Some(Self { commands, updates })
    }

    /// Publish the current state. The thread ignores duplicates in the setters.
    pub fn publish(&self, snapshot: MprisSnapshot) {
        let _ = self.updates.send(snapshot);
    }

    pub fn try_recv(&self) -> Option<MprisCommand> {
        self.commands.try_recv().ok()
    }
}

fn run(sender: CommandSender, update_rx: tokio::sync::watch::Receiver<MprisSnapshot>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("MusicWasp: MPRIS runtime failed: {e}");
            return;
        }
    };

    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let player = match Player::builder("musicwasp")
            .identity("MusicWasp")
            .desktop_entry("musicwasp")
            .can_quit(true)
            .can_raise(true)
            .can_play(true)
            .can_pause(true)
            .can_seek(true)
            .can_go_next(true)
            .can_go_previous(true)
            .build()
            .await
        {
            Ok(player) => player,
            Err(e) => {
                // No session bus (headless, CI, ssh): media keys fall back to
                // the compositor's own handling and the app still works.
                eprintln!("MusicWasp: MPRIS unavailable: {e}");
                return;
            }
        };

        connect_commands(&player, sender);

        // The server task must run for methods and property calls to arrive.
        tokio::task::spawn_local(player.run());

        let mut update_rx = update_rx;
        while update_rx.changed().await.is_ok() {
            let snapshot = update_rx.borrow_and_update().clone();
            apply_snapshot(&player, &snapshot).await;
        }
    });
}

fn connect_commands(player: &Player, command_tx: CommandSender) {
    let tx = command_tx.clone();
    player.connect_raise(move |_| {
        tx.send(MprisCommand::Raise);
    });

    let tx = command_tx.clone();
    player.connect_quit(move |_| {
        tx.send(MprisCommand::Quit);
    });

    let tx = command_tx.clone();
    player.connect_play(move |_| {
        tx.send(MprisCommand::Play);
    });

    let tx = command_tx.clone();
    player.connect_pause(move |_| {
        tx.send(MprisCommand::Pause);
    });

    let tx = command_tx.clone();
    player.connect_play_pause(move |_| {
        tx.send(MprisCommand::PlayPause);
    });

    let tx = command_tx.clone();
    player.connect_stop(move |_| {
        tx.send(MprisCommand::Stop);
    });

    let tx = command_tx.clone();
    player.connect_next(move |_| {
        tx.send(MprisCommand::Next);
    });

    let tx = command_tx.clone();
    player.connect_previous(move |_| {
        tx.send(MprisCommand::Previous);
    });

    let tx = command_tx.clone();
    player.connect_seek(move |_, offset| {
        tx.send(MprisCommand::Seek(offset.as_micros() as f64 / 1_000_000.0));
    });

    let tx = command_tx.clone();
    player.connect_set_position(move |_, _track, position| {
        tx.send(MprisCommand::SetPosition(
            position.as_micros() as f64 / 1_000_000.0,
        ));
    });

    let tx = command_tx.clone();
    player.connect_set_volume(move |_, volume| {
        tx.send(MprisCommand::SetVolume(volume as f32));
    });

    let tx = command_tx.clone();
    player.connect_set_shuffle(move |_, shuffle| {
        tx.send(MprisCommand::SetShuffle(shuffle));
    });

    let tx = command_tx;
    player.connect_set_loop_status(move |_, status| {
        let mode = match status {
            LoopStatus::None => LoopMode::None,
            LoopStatus::Track => LoopMode::Track,
            LoopStatus::Playlist => LoopMode::Playlist,
        };
        tx.send(MprisCommand::SetLoop(mode));
    });
}

/// Push the UI snapshot into the MPRIS properties. `mpris-server` ignores
/// setters whose value did not change, so repeated snapshots are cheap.
async fn apply_snapshot(player: &Player, snapshot: &MprisSnapshot) {
    let status = match snapshot.status {
        MprisStatus::Playing => PlaybackStatus::Playing,
        MprisStatus::Paused => PlaybackStatus::Paused,
        MprisStatus::Stopped => PlaybackStatus::Stopped,
    };
    let _ = player.set_playback_status(status).await;

    let mut metadata = Metadata::builder();
    if let Some(id) = snapshot.track_id {
        if let Ok(track_id) =
            TrackId::try_from(format!("/org/mpris/MediaPlayer2/musicwasp/track/{id}"))
        {
            metadata = metadata.trackid(track_id);
        }
    }
    if !snapshot.title.is_empty() {
        metadata = metadata.title(snapshot.title.as_str());
        if !snapshot.artist.is_empty() {
            metadata = metadata.artist([snapshot.artist.clone()]);
        }
        if !snapshot.album.is_empty() {
            metadata = metadata.album(snapshot.album.as_str());
        }
        if snapshot.duration_secs > 0.0 {
            metadata = metadata.length(Time::from_micros(
                (snapshot.duration_secs * 1_000_000.0) as i64,
            ));
        }
        let _ = player.set_metadata(metadata.build()).await;
    } else {
        let _ = player.set_metadata(Metadata::new()).await;
    }

    player.set_position(Time::from_micros(
        (snapshot.position_secs.max(0.0) * 1_000_000.0) as i64,
    ));
    let _ = player
        .set_volume(snapshot.volume.clamp(0.0, 1.5) as f64)
        .await;
    let _ = player.set_shuffle(snapshot.shuffle).await;
    let _ = player
        .set_loop_status(match snapshot.loop_mode {
            LoopMode::None => LoopStatus::None,
            LoopMode::Track => LoopStatus::Track,
            LoopMode::Playlist => LoopStatus::Playlist,
        })
        .await;
    let _ = player.set_can_go_next(snapshot.can_next).await;
    let _ = player.set_can_go_previous(snapshot.can_previous).await;
    let _ = player.set_can_play(true).await;
    let _ = player
        .set_can_pause(snapshot.status != MprisStatus::Stopped)
        .await;
    let _ = player.set_can_seek(snapshot.duration_secs > 0.0).await;
}

/// Poll interval for MPRIS position updates while playing.
pub const PLAYING_REFRESH: Duration = Duration::from_millis(250);
/// Poll interval when paused or stopped.
pub const IDLE_REFRESH: Duration = Duration::from_secs(1);
