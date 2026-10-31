//! Playback engine.
//!
//! A dedicated audio thread owns the output device and sink. The UI thread
//! sends commands and reads lock-free snapshots, so the GUI never blocks on
//! the audio device.

use crate::dsp::{EqShared, Equalizer};
use anyhow::Result;
use std::collections::VecDeque;
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
    replaygain_enabled: AtomicBool,
    replaygain_album: AtomicBool,
    current: Mutex<Option<PathBuf>>,
    /// Paths gapless playback moved to, consumed by the UI.
    advanced: Mutex<VecDeque<PathBuf>>,
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
        /// Queue a source after the current one for gapless transitions.
        Enqueue(PathBuf),
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
            shared.replaygain_enabled.store(true, Ordering::Relaxed);

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

        /// Append the next track to the sink so it starts without a gap.
        pub fn enqueue_next(&self, path: &Path) {
            let _ = self.tx.send(Cmd::Enqueue(path.to_path_buf()));
        }

        /// Enable/disable ReplayGain and choose album gain over track gain.
        pub fn set_replaygain(&self, enabled: bool, album: bool) {
            self.shared
                .replaygain_enabled
                .store(enabled, Ordering::Relaxed);
            self.shared.replaygain_album.store(album, Ordering::Relaxed);
        }

        /// Gapless playback advanced to this path. Consumed by the UI.
        pub fn take_advanced(&self) -> Option<PathBuf> {
            self.shared
                .advanced
                .lock()
                .ok()
                .and_then(|mut advanced| advanced.pop_front())
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
        let mut queued: VecDeque<(PathBuf, Duration)> = VecDeque::new();
        let mut current_rate: Option<u32> = None;
        let mut last_len = 0usize;

        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Cmd::Load(path)) => {
                    sink.stop();
                    queued.clear();
                    current_rate = None;
                    sink.set_volume(f32::from_bits(shared.volume.load(Ordering::Relaxed) as u32));
                    match open_source(&path, &shared) {
                        Ok((source, duration, rate)) => {
                            shared
                                .duration_ms
                                .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                            *shared.current.lock().unwrap() = Some(path);
                            let source = DspSource::new(source, Arc::clone(&shared.eq));
                            sink.append(source);
                            sink.play();
                            was_playing = true;
                            current_rate = Some(rate);
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
                    last_len = sink.len();
                }
                Ok(Cmd::Enqueue(path)) => {
                    let current = shared
                        .current
                        .lock()
                        .ok()
                        .and_then(|current| current.clone());
                    let duplicate = current.as_deref() == Some(path.as_path())
                        || queued.iter().any(|(queued_path, _)| queued_path == &path);
                    if !duplicate && current.is_some() && !sink.empty() {
                        match open_source(&path, &shared) {
                            Ok((source, duration, rate)) if current_rate == Some(rate) => {
                                let source = DspSource::new(source, Arc::clone(&shared.eq));
                                sink.append(source);
                                queued.push_back((path, duration));
                            }
                            Ok(_) => {
                                tracing::debug!("gapless skipped: sample rate changes");
                            }
                            Err(e) => {
                                tracing::warn!("cannot queue {} for gapless: {e}", path.display());
                            }
                        }
                    }
                    last_len = sink.len();
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
                    queued.clear();
                    current_rate = None;
                    was_playing = false;
                    shared
                        .status
                        .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                    shared.position_ms.store(0, Ordering::Relaxed);
                    shared.duration_ms.store(0, Ordering::Relaxed);
                    *shared.current.lock().unwrap() = None;
                    last_len = 0;
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

            let len = sink.len();
            if len < last_len {
                if let Some((path, duration)) = queued.pop_front() {
                    // The queued source is now playing: keep the snapshot and
                    // tell the UI which path became current.
                    shared
                        .duration_ms
                        .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                    *shared.current.lock().unwrap() = Some(path.clone());
                    if let Ok(mut advanced) = shared.advanced.lock() {
                        advanced.push_back(path);
                    }
                }
            }
            last_len = len;

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

    fn open_source(path: &Path, shared: &Shared) -> Result<(BoxedSource, Duration, u32)> {
        let file = BufReader::new(File::open(path)?);
        let decoder = Decoder::new(file)?;
        let duration = decoder
            .total_duration()
            .unwrap_or_else(|| Duration::from_secs(0));
        let sample_rate = decoder.sample_rate();

        let gain = if shared.replaygain_enabled.load(Ordering::Relaxed) {
            replaygain_gain(path, shared.replaygain_album.load(Ordering::Relaxed))
        } else {
            1.0
        };
        let source = decoder.amplify(gain);
        Ok((Box::new(source), duration, sample_rate))
    }

    /// ReplayGain gain factor from the file's track or album gain tag.
    fn replaygain_gain(path: &Path, prefer_album: bool) -> f32 {
        use lofty::file::TaggedFileExt;
        use lofty::prelude::ItemKey;
        use lofty::probe::Probe;

        let Ok(tagged) = Probe::open(path).and_then(|probe| probe.read()) else {
            return 1.0;
        };
        let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
            return 1.0;
        };
        let track = tag.get_string(&ItemKey::ReplayGainTrackGain);
        let album = tag.get_string(&ItemKey::ReplayGainAlbumGain);
        let raw = if prefer_album {
            album.or(track)
        } else {
            track.or(album)
        };
        let Some(raw) = raw else {
            return 1.0;
        };
        parse_replaygain(raw).unwrap_or(1.0)
    }

    pub(crate) fn parse_replaygain(raw: &str) -> Option<f32> {
        let cleaned = raw
            .trim()
            .trim_end_matches("dB")
            .trim_end_matches("db")
            .trim();
        cleaned
            .parse::<f32>()
            .ok()
            .map(|db| 10f32.powf(db / 20.0).clamp(0.1, 4.0))
    }
}

#[cfg(all(test, feature = "audio"))]
mod tests {
    use super::audio_impl::parse_replaygain;

    #[test]
    fn replaygain_db_is_converted_to_a_factor() {
        let half = parse_replaygain("-6.00 dB").unwrap();
        assert!((half - 0.501).abs() < 0.01);
        let boost = parse_replaygain("3 dB").unwrap();
        assert!((boost - 1.413).abs() < 0.01);
        assert!(parse_replaygain("").is_none());
        assert!(parse_replaygain("not a gain").is_none());
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
        pub fn enqueue_next(&self, _path: &Path) {}
        pub fn take_advanced(&self) -> Option<PathBuf> {
            None
        }
        pub fn set_replaygain(&self, _enabled: bool, _album: bool) {}
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
