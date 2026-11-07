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
    /// Crossfade length in seconds, stored as f32 bits.
    crossfade_secs: AtomicU64,
    current: Mutex<Option<PathBuf>>,
    /// Paths gapless playback moved to, consumed by the UI.
    advanced: Mutex<VecDeque<PathBuf>>,
    eq: Arc<EqShared>,
}

#[cfg(feature = "audio")]
pub use audio_impl::Player;

/// Names of the system's output devices, for the settings dropdown.
#[cfg(feature = "audio")]
pub fn output_devices() -> Vec<String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    let host = rodio::cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    let mut names: Vec<String> = devices.filter_map(|device| device.name().ok()).collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(not(feature = "audio"))]
pub fn output_devices() -> Vec<String> {
    Vec::new()
}

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
    use std::time::{Duration, Instant};

    enum Cmd {
        Load(PathBuf),
        /// Stream a radio station or podcast episode over HTTP.
        LoadUrl(String),
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
        tap: Arc<crate::analysis::AudioTap>,
        _join: thread::JoinHandle<()>,
    }

    impl Player {
        pub fn new() -> Result<Self> {
            Self::new_with_device(None)
        }

        /// Open a specific output device, or the system default when `None`.
        pub fn new_with_device(device: Option<String>) -> Result<Self> {
            let shared = Arc::new(Shared::default());
            shared
                .volume
                .store(0.8f32.to_bits() as u64, Ordering::Relaxed);
            shared.replaygain_enabled.store(true, Ordering::Relaxed);

            let (tx, rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let shared_thread = Arc::clone(&shared);
            let tap = crate::analysis::AudioTap::new();
            let tap_thread = Arc::clone(&tap);

            let join =
                thread::spawn(move || audio_loop(rx, shared_thread, tap_thread, ready_tx, device));

            match ready_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => anyhow::bail!("{e}"),
                Err(_) => anyhow::bail!("audio thread died during init"),
            }

            Ok(Self {
                tx,
                shared,
                tap,
                _join: join,
            })
        }

        /// Post-EQ PCM for visualizers.
        pub fn audio_tap(&self) -> Arc<crate::analysis::AudioTap> {
            Arc::clone(&self.tap)
        }

        pub fn load_and_play(&self, path: &Path) {
            let _ = self.tx.send(Cmd::Load(path.to_path_buf()));
        }

        /// Play an internet radio or podcast stream.
        pub fn load_url(&self, url: &str) {
            let _ = self.tx.send(Cmd::LoadUrl(url.to_string()));
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

        /// Crossfade seconds; 0 disables overlap and keeps gapless joining.
        pub fn set_crossfade(&self, secs: f32) {
            self.shared
                .crossfade_secs
                .store(secs.max(0.0).to_bits() as u64, Ordering::Relaxed);
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

    struct Fade {
        from: usize,
        to: usize,
        started: Instant,
        secs: f32,
        next: (PathBuf, Duration),
    }

    fn crossfade_secs(shared: &Shared) -> f32 {
        f32::from_bits(shared.crossfade_secs.load(Ordering::Relaxed) as u32).max(0.0)
    }

    fn set_mix_volumes(sinks: &[Sink; 2], fade: Option<&Fade>, volume: f32) {
        if let Some(fade) = fade {
            let progress = (fade.started.elapsed().as_secs_f32() / fade.secs).clamp(0.0, 1.0);
            sinks[fade.from].set_volume(volume * (1.0 - progress));
            sinks[fade.to].set_volume(volume * progress);
        } else {
            sinks[0].set_volume(volume);
            sinks[1].set_volume(0.0);
        }
    }

    fn audio_loop(
        rx: Receiver<Cmd>,
        shared: Arc<Shared>,
        tap: Arc<crate::analysis::AudioTap>,
        ready: Sender<Result<(), String>>,
        device_name: Option<String>,
    ) {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};

        let host = rodio::cpal::default_host();
        let device = match device_name.as_deref() {
            Some(name) => host.output_devices().ok().and_then(|mut devices| {
                devices.find(|device| device.name().ok().as_deref() == Some(name))
            }),
            None => host.default_output_device(),
        };
        let Some(device) = device else {
            let _ = ready.send(Err(match &device_name {
                Some(name) => format!("output device not found: {name}"),
                None => "no audio output device".to_string(),
            }));
            return;
        };

        let (_stream, handle) = match OutputStream::try_from_device(&device) {
            Ok(stream) => stream,
            Err(e) => {
                let _ = ready.send(Err(format!("cannot open audio output: {e}")));
                return;
            }
        };

        let (first, second) = match (Sink::try_new(&handle), Sink::try_new(&handle)) {
            (Ok(first), Ok(second)) => (first, second),
            (Err(e), _) | (_, Err(e)) => {
                let _ = ready.send(Err(format!("sink init failed: {e}")));
                return;
            }
        };
        let sinks = [first, second];
        sinks[0].set_volume(0.8);
        sinks[1].pause();
        sinks[1].set_volume(0.0);
        let _ = ready.send(Ok(()));

        let mut active = 0usize;
        let mut was_playing = false;
        let mut queued: VecDeque<(PathBuf, Duration)> = VecDeque::new();
        let mut current: Option<(PathBuf, Duration)> = None;
        let mut current_rate: Option<u32> = None;
        let mut pending: Option<(PathBuf, Duration)> = None;
        let mut fade: Option<Fade> = None;
        let mut last_len = 0usize;

        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Cmd::Load(path)) => {
                    for sink in &sinks {
                        sink.stop();
                    }
                    sinks[1].pause();
                    sinks[1].set_volume(0.0);
                    queued.clear();
                    pending = None;
                    fade = None;
                    current = None;
                    current_rate = None;
                    active = 0;
                    let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed) as u32);
                    sinks[0].set_volume(volume);
                    match open_source(&path, &shared) {
                        Ok((source, duration, rate)) => {
                            shared
                                .duration_ms
                                .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                            *shared.current.lock().unwrap() = Some(path.clone());
                            tap.set_format(rate, source.channels());
                            tap.clear();
                            sinks[0].append(DspSource::new(
                                source,
                                Arc::clone(&shared.eq),
                                Some(Arc::clone(&tap)),
                            ));
                            sinks[0].play();
                            was_playing = true;
                            current_rate = Some(rate);
                            current = Some((path, duration));
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
                    last_len = sinks[active].len();
                }
                Ok(Cmd::LoadUrl(url)) => {
                    for sink in &sinks {
                        sink.stop();
                    }
                    sinks[1].pause();
                    sinks[1].set_volume(0.0);
                    queued.clear();
                    pending = None;
                    fade = None;
                    current = None;
                    current_rate = None;
                    active = 0;
                    *shared.current.lock().unwrap() = None;
                    shared.duration_ms.store(0, Ordering::Relaxed);
                    shared.position_ms.store(0, Ordering::Relaxed);
                    let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed) as u32);
                    sinks[0].set_volume(volume);
                    match open_url(&url) {
                        Ok((source, rate)) => {
                            *shared.current.lock().unwrap() = Some(PathBuf::from(url.clone()));
                            shared.duration_ms.store(0, Ordering::Relaxed);
                            shared.position_ms.store(0, Ordering::Relaxed);
                            tap.set_format(rate, source.channels());
                            tap.clear();
                            sinks[0].append(DspSource::new(
                                source,
                                Arc::clone(&shared.eq),
                                Some(Arc::clone(&tap)),
                            ));
                            sinks[0].play();
                            was_playing = true;
                            current_rate = Some(rate);
                            current = Some((PathBuf::from(url), Duration::ZERO));
                            shared
                                .status
                                .store(PlayerStatus::Playing.to_u8(), Ordering::Relaxed);
                            shared.finished.store(false, Ordering::Relaxed);
                        }
                        Err(e) => {
                            tracing::warn!("cannot stream {url}: {e}");
                            shared
                                .status
                                .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                        }
                    }
                    last_len = sinks[active].len();
                }
                Ok(Cmd::Enqueue(path)) => {
                    let duplicate = current
                        .as_ref()
                        .is_some_and(|(current, _)| current == &path)
                        || pending.as_ref().is_some_and(|(queued, _)| queued == &path)
                        || queued.iter().any(|(queued_path, _)| queued_path == &path);
                    if !duplicate && current.is_some() && !sinks[active].empty() {
                        let crossfade = crossfade_secs(&shared);
                        match open_source(&path, &shared) {
                            Ok((source, duration, rate)) if current_rate == Some(rate) => {
                                if crossfade > 0.1
                                    && fade.is_none()
                                    && pending.is_none()
                                    && queued.is_empty()
                                {
                                    let inactive = 1 - active;
                                    sinks[inactive].stop();
                                    tap.set_format(rate, source.channels());
                                    sinks[inactive].append(DspSource::new(
                                        source,
                                        Arc::clone(&shared.eq),
                                        Some(Arc::clone(&tap)),
                                    ));
                                    sinks[inactive].pause();
                                    sinks[inactive].set_volume(0.0);
                                    pending = Some((path, duration));
                                } else if fade.is_none() {
                                    tap.set_format(rate, source.channels());
                                    sinks[active].append(DspSource::new(
                                        source,
                                        Arc::clone(&shared.eq),
                                        Some(Arc::clone(&tap)),
                                    ));
                                    queued.push_back((path, duration));
                                }
                            }
                            Ok(_) => {
                                tracing::debug!("transition skipped: sample rate changes");
                            }
                            Err(e) => {
                                tracing::warn!("cannot queue {}: {e}", path.display());
                            }
                        }
                    }
                    last_len = sinks[active].len();
                }
                Ok(Cmd::Resume) => {
                    if let Some(fade) = &fade {
                        sinks[fade.from].play();
                        sinks[fade.to].play();
                    } else {
                        sinks[active].play();
                    }
                    was_playing = true;
                    shared
                        .status
                        .store(PlayerStatus::Playing.to_u8(), Ordering::Relaxed);
                }
                Ok(Cmd::Pause) => {
                    if let Some(fade) = &fade {
                        sinks[fade.from].pause();
                        sinks[fade.to].pause();
                    } else {
                        sinks[active].pause();
                    }
                    was_playing = false;
                    shared
                        .status
                        .store(PlayerStatus::Paused.to_u8(), Ordering::Relaxed);
                }
                Ok(Cmd::Stop) => {
                    for sink in &sinks {
                        sink.stop();
                    }
                    tap.clear();
                    queued.clear();
                    pending = None;
                    fade = None;
                    current = None;
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
                    if let Err(e) = sinks[active].try_seek(target) {
                        tracing::warn!("seek failed: {e}");
                    }
                }
                Ok(Cmd::Volume(v)) => {
                    shared.volume.store(v.to_bits() as u64, Ordering::Relaxed);
                    set_mix_volumes(&sinks, fade.as_ref(), v);
                }
                Ok(Cmd::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed) as u32);
            let crossfade = crossfade_secs(&shared);

            // Begin an overlap when the current source is close enough to its end.
            if fade.is_none() && crossfade > 0.1 {
                if let (Some((_, duration)), Some(_)) = (&current, &pending) {
                    let position = sinks[active].get_pos().as_secs_f32();
                    let duration = duration.as_secs_f32();
                    if duration > 0.0 && duration - position <= crossfade {
                        let inactive = 1 - active;
                        sinks[inactive].set_volume(0.0);
                        sinks[inactive].play();
                        let next = pending.take().expect("pending checked above");
                        fade = Some(Fade {
                            from: active,
                            to: inactive,
                            started: Instant::now(),
                            secs: crossfade,
                            next,
                        });
                    }
                }
            }

            if let Some(active_fade) = &fade {
                let progress = (active_fade.started.elapsed().as_secs_f32() / active_fade.secs)
                    .clamp(0.0, 1.0);
                sinks[active_fade.from].set_volume(volume * (1.0 - progress));
                sinks[active_fade.to].set_volume(volume * progress);
            }

            // Finish a crossfade and publish the new current track.
            let finished_fade = fade
                .as_ref()
                .is_some_and(|fade| fade.started.elapsed().as_secs_f32() >= fade.secs);
            if finished_fade {
                let fade = fade.take().expect("fade checked above");
                sinks[fade.from].stop();
                sinks[fade.from].pause();
                sinks[fade.from].set_volume(0.0);
                active = fade.to;
                sinks[active].set_volume(volume);
                let (path, duration) = fade.next;
                shared
                    .duration_ms
                    .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                *shared.current.lock().unwrap() = Some(path.clone());
                current = Some((path.clone(), duration));
                if let Ok(mut advanced) = shared.advanced.lock() {
                    advanced.push_back(path);
                }
                last_len = sinks[active].len();
            }

            shared.position_ms.store(
                sinks[active].get_pos().as_millis() as u64,
                Ordering::Relaxed,
            );

            let len = sinks[active].len();
            if fade.is_none() && len < last_len {
                if let Some((path, duration)) = queued.pop_front() {
                    shared
                        .duration_ms
                        .store((duration.as_secs_f64() * 1000.0) as u64, Ordering::Relaxed);
                    *shared.current.lock().unwrap() = Some(path.clone());
                    current = Some((path.clone(), duration));
                    if let Ok(mut advanced) = shared.advanced.lock() {
                        advanced.push_back(path);
                    }
                }
            }
            if fade.is_none() {
                last_len = len;
            }

            if fade.is_none() && sinks[active].empty() && was_playing && pending.is_none() {
                was_playing = false;
                shared
                    .status
                    .store(PlayerStatus::Stopped.to_u8(), Ordering::Relaxed);
                shared.finished.store(true, Ordering::Relaxed);
            }
        }
    }

    type BoxedSource = Box<dyn Source<Item = i16> + Send + 'static>;

    /// HTTP stream with a small rewind cache. Rodio probes the first bytes of
    /// a stream by seeking back to the start; live streams cannot seek, so the
    /// cache makes the probe work while keeping the stream sequential.
    struct HttpStream {
        response: reqwest::blocking::Response,
        cache: Vec<u8>,
        position: u64,
    }

    const STREAM_CACHE_LIMIT: usize = 1 << 20;

    impl std::io::Read for HttpStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let position = self.position as usize;
            if position < self.cache.len() {
                let available = &self.cache[position..];
                let count = available.len().min(buffer.len());
                buffer[..count].copy_from_slice(&available[..count]);
                self.position += count as u64;
                return Ok(count);
            }
            let count = self.response.read(buffer)?;
            if self.cache.len() < STREAM_CACHE_LIMIT {
                let room = STREAM_CACHE_LIMIT - self.cache.len();
                self.cache.extend_from_slice(&buffer[..count.min(room)]);
            }
            self.position += count as u64;
            Ok(count)
        }
    }

    impl std::io::Seek for HttpStream {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            let target = match position {
                std::io::SeekFrom::Start(value) => value,
                std::io::SeekFrom::Current(delta) => self.position.saturating_add_signed(delta),
                std::io::SeekFrom::End(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "live streams have no known end",
                    ));
                }
            };
            if target <= self.cache.len() as u64 {
                self.position = target;
                Ok(target)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "cannot seek past the buffered part of a live stream",
                ))
            }
        }
    }

    fn open_url(url: &str) -> Result<(BoxedSource, u32)> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(crate::autotag::USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        let response = client.get(url).send()?.error_for_status()?;
        let decoder = Decoder::new(HttpStream {
            response,
            cache: Vec::new(),
            position: 0,
        })?;
        let sample_rate = decoder.sample_rate();
        Ok((Box::new(decoder), sample_rate))
    }

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
        /// Post-EQ PCM for visualizers. The silent engine never has audio.
        pub fn audio_tap(&self) -> Arc<crate::analysis::AudioTap> {
            crate::analysis::AudioTap::new()
        }
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
