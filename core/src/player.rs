//! Playback engine.
//!
//! A dedicated audio thread owns the output device and sink. The UI thread
//! sends commands and reads lock-free snapshots, so the GUI never blocks on
//! the audio device.

use crate::dsp::{EqShared, Equalizer};
use anyhow::Result;
use std::path::{Path, PathBuf};
#[cfg(feature = "audio")]
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerStatus {
    Stopped,
    Playing,
    Paused,
}

impl PlayerStatus {
    fn to_u8(self) -> u8 {
        match self {
            PlayerStatus::Stopped => 0,
            PlayerStatus::Playing => 1,
            PlayerStatus::Paused => 2,
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            1 => PlayerStatus::Playing,
            2 => PlayerStatus::Paused,
            _ => PlayerStatus::Stopped,
        }
    }
}

/// Point-in-time view of the player, cheap to clone every frame.
#[derive(Debug, Clone)]
pub struct PlayerSnapshot {
    pub status: PlayerStatus,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub current: Option<PathBuf>,
    /// Set when a track ran to its end; cleared by [`Player::take_finished`].
    pub finished: bool,
}

#[derive(Default)]
struct Shared {
    status: AtomicU8,
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    volume: AtomicU64, // f32 bits
    finished: AtomicBool,
    current: Mutex<Option<PathBuf>>,
    eq: Arc<EqShared>,
}

#[cfg(feature = "audio")]
pub use audio_impl::Player;

#[cfg(not(feature = "audio"))]
pub use silent_impl::Player;

/// Shared handle to the playback engine.
#[cfg(feature = "audio")]
mod audio_impl {
    use super::*;
    use crate::dsp::DspSource;
    use rodio::{Decoder, OutputStream, Sink, Source};
    use std::fs::File;
    use std::io::BufReader;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    enum Cmd {
        Load(PathBuf),
        Resume,
        Pause,
        Stop,
        Seek(f64),
        Volume(f32),
        Shutdown,
    }

    pub struct Player {
        tx: Sender<Cmd>,
        shared: Arc<Shared>,
        _join: thread::JoinHandle<()>,
    }

    impl Player {
        pub fn new() -> Result<Self> {
            let shared = Arc::new(Shared::default());
            shared
                .volume
                .store(0.8f32.to_bits() as u64, Ordering::Relaxed);

            let (tx, rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let shared_thread = Arc::clone(&shared);

            let join = thread::spawn(move || audio_loop(rx, shared_thread, ready_tx));

            match ready_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => anyhow::bail!("{e}"),
                Err(_) => anyhow::bail!("audio thread died during init"),
            }

            Ok(Self {
                tx,
                shared,
                _join: join,
            })
        }

        pub fn load_and_play(&self, path: &Path) {
            let _ = self.tx.send(Cmd::Load(path.to_path_buf()));
        }

        pub fn toggle_pause(&self) {
            match self.snapshot().status {
                PlayerStatus::Playing => self.pause(),
                PlayerStatus::Paused => self.resume(),
                PlayerStatus::Stopped => {}
            }
        }

        pub fn pause(&self) {
            let _ = self.tx.send(Cmd::Pause);
        }

        pub fn resume(&self) {
            let _ = self.tx.send(Cmd::Resume);
        }

        pub fn stop(&self) {
            let _ = self.tx.send(Cmd::Stop);
        }

        pub fn seek(&self, secs: f64) {
            let _ = self.tx.send(Cmd::Seek(secs));
        }

        pub fn set_volume(&self, v: f32) {
            let _ = self.tx.send(Cmd::Volume(v.clamp(0.0, 1.5)));
        }

        /// Apply EQ settings to the running track without restarting it.
        pub fn set_equalizer(&self, settings: Equalizer) {
            self.shared.eq.set(settings);
        }

        pub fn snapshot(&self) -> PlayerSnapshot {
            PlayerSnapshot {
                status: PlayerStatus::from_u8(self.shared.status.load(Ordering::Relaxed)),
                position: self.shared.position_ms.load(Ordering::Relaxed) as f64 / 1000.0,
                duration: self.shared.duration_ms.load(Ordering::Relaxed) as f64 / 1000.0,
                volume: f32::from_bits(self.shared.volume.load(Ordering::Relaxed) as u32),
                current: self.shared.current.lock().ok().and_then(|g| g.clone()),
                finished: self.shared.finished.load(Ordering::Relaxed),
            }
        }

        /// Returns true once per finished track, then resets the flag.
        pub fn take_finished(&self) -> bool {
            self.shared.finished.swap(false, Ordering::Relaxed)
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            let _ = self.tx.send(Cmd::Shutdown);
        }
    }

    fn audio_loop(rx: Receiver<Cmd>, shared: Arc<Shared>, ready: Sender<Result<(), String>>) {
        let (_stream, handle) = match OutputStream::try_default() {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(format!("cannot open audio output: {e}")));
                return;
            }
        };

        let sink = match Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("sink init failed: {e}");
                return;
            }
        };
        sink.set_volume(0.8);

        let mut was_playing = false;

        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Cmd::Load(path)) => {
                    sink.stop();
                    sink.set_volume(f32::from_bits(shared.volume.load(Ordering::Relaxed) as u32));
                    match open_source(&path) {
                        Ok((source, duration)) => {
                            shared
                                .duration_ms
                                .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                            *shared.current.lock().unwrap() = Some(path);
                            let source = DspSource::new(source, Arc::clone(&shared.eq));
                            sink.append(source);
                            sink.play();
                            was_playing = true;
                            shared
                                .status
                                .store(PlayerStatus::Playing.to_u8(), Ordering::Relaxed);
                            shared.finished.store(false, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::warn!("cannot play {}: {e}", path.display());
                            shared
                                .status
                                .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                        }
                    }
                }
                Ok(Cmd::Resume) => {
                    sink.play();
                    was_playing = true;
                    shared
                        .status
                        .store(PlayerStatus::Playing.to_u8(), Ordering::Relaxed);
                }
                Ok(Cmd::Pause) => {
                    sink.pause();
                    was_playing = false;
                    shared
                        .status
                        .store(PlayerStatus::Paused.to_u8(), Ordering::Relaxed);
                }
                Ok(Cmd::Stop) => {
                    sink.stop(); // also clears the queue
                    was_playing = false;
                    shared
                        .status
                        .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                    shared.position_ms.store(0, Ordering::Relaxed);
                    shared.duration_ms.store(0, Ordering::Relaxed);
                    *shared.current.lock().unwrap() = None;
                }
                Ok(Cmd::Seek(secs)) => {
                    let target = Duration::from_secs_f64(secs.max(0.0));
                    if let Err(e) = sink.try_seek(target) {
                        tracing::warn!("seek failed: {e}");
                    }
                }
                Ok(Cmd::Volume(v)) => {
                    shared.volume.store(v.to_bits() as u64, Ordering::Relaxed);
                    sink.set_volume(v);
                }
                Ok(Cmd::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            // Keep the snapshot fresh and detect end-of-track.
            shared
                .position_ms
                .store(sink.get_pos().as_millis() as u64, Ordering::Relaxed);

            if sink.empty() && was_playing {
                was_playing = false;
                shared
                    .status
                    .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                shared.finished.store(true, Ordering::Relaxed);
            }
        }
    }

    type BoxedSource = Box<dyn Source<Item = i16> + Send + 'static>;

    fn open_source(path: &Path) -> Result<(BoxedSource, Duration)> {
        let file = BufReader::new(File::open(path)?);
        let decoder = Decoder::new(file)?;
        let duration = decoder
            .total_duration()
            .unwrap_or_else(|| Duration::from_secs(0));
        Ok((Box::new(decoder), duration))
    }
}

#[cfg(not(feature = "audio"))]
mod silent_impl {
    use super::*;

    pub struct Player;

    impl Player {
        pub fn new() -> Result<Self> {
            anyhow::bail!("built without audio support")
        }
        pub fn load_and_play(&self, _path: &Path) {}
        pub fn toggle_pause(&self) {}
        pub fn pause(&self) {}
        pub fn resume(&self) {}
        pub fn stop(&self) {}
        pub fn seek(&self, _secs: f64) {}
        pub fn set_volume(&self, _v: f32) {}
        pub fn set_equalizer(&self, _settings: Equalizer) {}
        pub fn take_finished(&self) -> bool {
            false
        }
        pub fn snapshot(&self) -> PlayerSnapshot {
            PlayerSnapshot {
                status: PlayerStatus::Stopped,
                position: 0.0,
                duration: 0.0,
                volume: 0.0,
                current: None,
                finished: false,
            }
        }
    }
}
