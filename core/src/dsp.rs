//! Equalizer / DSP.
//!
//! A 10-band peaking equalizer with a preamp. The filter chain is deliberately
//! small and allocation-free once built: [`EqProcessor`] runs on the audio
//! thread for every decoded sample. [`EqShared`] lets the UI change the
//! settings while a track is playing without restarting playback.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "audio")]
use std::sync::Arc;
use std::sync::Mutex;

/// Centre frequencies of the 10 ISO octave bands.
pub const BAND_FREQS: [f32; BAND_COUNT] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

pub const BAND_COUNT: usize = 10;

/// How far each band can be moved, in dB.
pub const GAIN_RANGE_DB: f32 = 12.0;
/// How far the preamp can be moved, in dB.
pub const PREAMP_RANGE_DB: f32 = 12.0;

/// Q of each band. Octave-spaced bands use ~1.41 so neighbours overlap gently.
const BAND_Q: f32 = 1.41;

/// A named equalizer preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub gains_db: [f32; BAND_COUNT],
    pub preamp_db: f32,
}

/// Presets are intentionally conservative: no +12 dB shelves that clip.
pub const PRESETS: [Preset; 8] = [
    Preset {
        name: "Flat",
        gains_db: [0.0; BAND_COUNT],
        preamp_db: 0.0,
    },
    Preset {
        name: "Rock",
        gains_db: [4.5, 3.5, 1.5, -1.5, -2.5, -1.0, 1.5, 3.0, 4.0, 4.5],
        preamp_db: -3.0,
    },
    Preset {
        name: "Pop",
        gains_db: [-1.0, -0.5, 1.0, 3.0, 4.0, 3.0, 1.0, -0.5, -1.0, -1.5],
        preamp_db: -2.0,
    },
    Preset {
        name: "Jazz",
        gains_db: [3.0, 2.0, 1.0, 1.5, -1.0, -1.5, 0.0, 1.0, 2.0, 2.5],
        preamp_db: -2.0,
    },
    Preset {
        name: "Classical",
        gains_db: [2.5, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, -1.0, 1.5, 2.0],
        preamp_db: -1.5,
    },
    Preset {
        name: "Bass boost",
        gains_db: [7.0, 6.0, 4.5, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        preamp_db: -4.0,
    },
    Preset {
        name: "Vocal",
        gains_db: [-3.0, -2.5, -2.0, -1.0, 2.0, 4.5, 4.0, 2.0, 0.0, -1.0],
        preamp_db: -2.0,
    },
    Preset {
        name: "Electronic",
        gains_db: [5.0, 4.0, 2.0, 0.0, -1.5, 0.0, 1.0, 2.0, 4.0, 5.0],
        preamp_db: -3.0,
    },
];

/// Equalizer state, persisted in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equalizer {
    pub enabled: bool,
    pub preamp_db: f32,
    pub gains_db: [f32; BAND_COUNT],
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            gains_db: [0.0; BAND_COUNT],
        }
    }
}

impl Equalizer {
    /// Levels clamped to the UI range.
    pub fn clamped(mut self) -> Self {
        self.preamp_db = self.preamp_db.clamp(-PREAMP_RANGE_DB, PREAMP_RANGE_DB);
        for gain in &mut self.gains_db {
            *gain = gain.clamp(-GAIN_RANGE_DB, GAIN_RANGE_DB);
        }
        self
    }

    /// True when this is exactly the flat preset.
    pub fn is_flat(&self) -> bool {
        self.gains_db.iter().all(|g| g.abs() < f32::EPSILON) && self.preamp_db.abs() < f32::EPSILON
    }

    /// Name of the matching preset, if the gains match one exactly.
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS
            .iter()
            .find(|preset| preset.preamp_db == self.preamp_db && preset.gains_db == self.gains_db)
            .map(|preset| preset.name)
    }

    /// Replace the gains and preamp with `preset`, keeping the enabled flag.
    pub fn apply_preset(&mut self, preset: &Preset) {
        self.gains_db = preset.gains_db;
        self.preamp_db = preset.preamp_db;
    }

    /// Magnitude response of the whole chain at `freq`, in dB.
    ///
    /// Used by the GUI to draw the EQ curve. Bypassed equalizers return 0 dB.
    pub fn response_db(&self, freq: f32, sample_rate: f32) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let mut db = self.preamp_db as f64;
        for (index, &gain) in self.gains_db.iter().enumerate() {
            if gain.abs() < f32::EPSILON {
                continue;
            }
            let coeffs = Coefficients::peaking(
                BAND_FREQS[index] as f64,
                gain as f64,
                BAND_Q as f64,
                sample_rate as f64,
            );
            db += coeffs.magnitude_db(freq as f64, sample_rate as f64);
        }
        db as f32
    }
}

/// Shared handle used by the audio thread and the UI.
///
/// `version` lets the running source avoid taking the mutex for every sample.
#[derive(Debug, Default)]
pub struct EqShared {
    settings: Mutex<Equalizer>,
    version: AtomicU64,
}

impl EqShared {
    pub fn new(settings: Equalizer) -> Self {
        Self {
            settings: Mutex::new(settings.clamped()),
            version: AtomicU64::new(1),
        }
    }

    pub fn set(&self, settings: Equalizer) {
        if let Ok(mut guard) = self.settings.lock() {
            *guard = settings.clamped();
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn snapshot(&self) -> Equalizer {
        match self.settings.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

/// Normalised biquad coefficients.
#[derive(Debug, Clone, Copy)]
struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    const BYPASS: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ cookbook peaking EQ.
    fn peaking(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        if sample_rate <= 0.0 || freq <= 0.0 || freq >= sample_rate / 2.0 {
            return Self::BYPASS;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// |H(e^jw)| converted to dB.
    fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * freq / sample_rate;
        // Evaluate H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)
        let (sin1, cos1) = (w.sin(), w.cos());
        let (sin2, cos2) = ((2.0 * w).sin(), (2.0 * w).cos());

        let num_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
        let num_im = -(self.b1 * sin1 + self.b2 * sin2);
        let den_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
        let den_im = -(self.a1 * sin1 + self.a2 * sin2);

        let num = (num_re * num_re + num_im * num_im).sqrt();
        let den = (den_re * den_re + den_im * den_im).sqrt();
        if den <= f64::EPSILON {
            return 0.0;
        }
        20.0 * (num / den).log10()
    }
}

#[derive(Debug, Clone, Copy)]
struct Biquad {
    coeffs: Coefficients,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(coeffs: Coefficients) -> Self {
        Self {
            coeffs,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn set_coeffs(&mut self, coeffs: Coefficients) {
        // Keep the history: changing a slider must not click.
        self.coeffs = coeffs;
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let c = self.coeffs;
        let y = (c.b0 * x as f64 + c.b1 * self.x1 as f64 + c.b2 * self.x2 as f64
            - c.a1 * self.y1 as f64
            - c.a2 * self.y2 as f64) as f32;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct ChannelChain {
    bands: [Biquad; BAND_COUNT],
}

/// Runs the equalizer over interleaved samples.
pub struct EqProcessor {
    sample_rate: f32,
    channels: usize,
    settings: Equalizer,
    preamp: f32,
    chains: Vec<ChannelChain>,
    frame: usize,
}

impl EqProcessor {
    pub fn new(sample_rate: u32, channels: u16, settings: Equalizer) -> Self {
        let sample_rate = sample_rate.max(1) as f32;
        let channels = channels.max(1) as usize;
        let settings = settings.clamped();
        let chains = (0..channels)
            .map(|_| ChannelChain {
                bands: std::array::from_fn(|_| Biquad::new(Coefficients::BYPASS)),
            })
            .collect();
        let mut processor = Self {
            sample_rate,
            channels,
            settings,
            preamp: 1.0,
            chains,
            frame: 0,
        };
        processor.rebuild();
        processor
    }

    pub fn settings(&self) -> Equalizer {
        self.settings
    }

    /// Swap in new settings; filter history is preserved where possible.
    pub fn update(&mut self, settings: Equalizer) {
        self.settings = settings.clamped();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.preamp = 10f32.powf(self.settings.preamp_db / 20.0);
        for chain in &mut self.chains {
            for (band, &gain) in self.settings.gains_db.iter().enumerate() {
                let coeffs = Coefficients::peaking(
                    BAND_FREQS[band] as f64,
                    gain as f64,
                    BAND_Q as f64,
                    self.sample_rate as f64,
                );
                chain.bands[band].set_coeffs(coeffs);
            }
        }
    }

    pub fn reset(&mut self) {
        for chain in &mut self.chains {
            for band in &mut chain.bands {
                band.reset();
            }
        }
        self.frame = 0;
    }

    /// Process one interleaved sample.
    #[inline]
    pub fn process(&mut self, sample: f32) -> f32 {
        if !self.settings.enabled {
            self.frame = (self.frame + 1) % self.channels;
            return sample;
        }

        let channel = self.frame;
        self.frame = (self.frame + 1) % self.channels;

        let mut out = sample * self.preamp;
        if let Some(chain) = self.chains.get_mut(channel) {
            for band in &mut chain.bands {
                out = band.process(out);
            }
        }
        // Filter ringing or a hot preamp must not wrap around into noise.
        out.clamp(-1.0, 1.0)
    }
}

/// A rodio source that runs the equalizer over the decoded samples.
#[cfg(feature = "audio")]
pub struct DspSource<S> {
    inner: S,
    shared: Arc<EqShared>,
    version: u64,
    processor: EqProcessor,
}

#[cfg(feature = "audio")]
impl<S> DspSource<S>
where
    S: rodio::Source<Item = i16>,
{
    pub fn new(inner: S, shared: Arc<EqShared>) -> Self {
        // Read the version first: a concurrent change after the snapshot is
        // then detected on the first sample instead of being missed.
        let version = shared.version();
        let settings = shared.snapshot();
        let processor = EqProcessor::new(inner.sample_rate(), inner.channels(), settings);
        Self {
            inner,
            version,
            processor,
            shared,
        }
    }
}

#[cfg(feature = "audio")]
impl<S> Iterator for DspSource<S>
where
    S: rodio::Source<Item = i16>,
{
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let version = self.shared.version();
        if version != self.version {
            self.version = version;
            self.processor.update(self.shared.snapshot());
        }
        self.inner.next().map(|sample| {
            let value = sample as f32 / 32768.0;
            (self.processor.process(value) * 32767.0) as i16
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[cfg(feature = "audio")]
impl<S> rodio::Source for DspSource<S>
where
    S: rodio::Source<Item = i16>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        // Old filter history belongs to the old position.
        self.processor.reset();
        self.inner.try_seek(pos)
    }
}

/// Frequency labels shown under the sliders.
pub fn band_label(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{:.0}k", freq / 1000.0)
    } else {
        format!("{freq:.0}")
    }
}

/// One-phrase description of what the current curve is doing.
pub fn describe(eq: &Equalizer) -> &'static str {
    if !eq.enabled {
        return "Bypassed";
    }
    if eq.is_flat() {
        return "Flat";
    }
    let bass: f32 = eq.gains_db[..4].iter().sum::<f32>() / 4.0;
    let treble: f32 = eq.gains_db[7..].iter().sum::<f32>() / 3.0;
    match bass - treble {
        d if d > 3.0 => "Bass-forward",
        d if d < -3.0 => "Treble-forward",
        _ => "Balanced",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn sine(freq: f32, sample_rate: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin() * 0.25)
            .collect()
    }

    #[test]
    fn flat_disabled_is_passthrough() {
        let eq = Equalizer::default();
        let mut processor = EqProcessor::new(48_000, 2, eq);
        for sample in [0.0, 0.25, -0.5, 1.0, -1.0] {
            assert_eq!(processor.process(sample), sample);
        }
    }

    #[test]
    fn enabled_flat_is_effectively_passthrough() {
        let eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        let mut processor = EqProcessor::new(48_000, 1, eq);
        let input = sine(1_000.0, 48_000.0, 4_800);
        let output: Vec<f32> = input.iter().map(|&s| processor.process(s)).collect();
        let diff = input
            .iter()
            .zip(&output)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(diff < 1e-6, "flat EQ altered the signal by {diff}");
    }

    #[test]
    fn boost_raises_the_matching_band() {
        let mut eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        eq.gains_db[5] = 9.0; // 1 kHz

        let mut processor = EqProcessor::new(48_000, 1, eq);
        let input = sine(1_000.0, 48_000.0, 24_000);
        // Discard the filter's start-up transient.
        let output: Vec<f32> = input[4_800..]
            .iter()
            .map(|&s| processor.process(s))
            .collect();
        let gain_db = 20.0 * (rms(&output) / rms(&input[4_800..])).log10();
        assert!(gain_db > 7.0, "+9 dB band only produced {gain_db:.1} dB");
    }

    #[test]
    fn cut_lowers_the_matching_band() {
        let mut eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        eq.gains_db[5] = -9.0;

        let mut processor = EqProcessor::new(48_000, 1, eq);
        let input = sine(1_000.0, 48_000.0, 24_000);
        let output: Vec<f32> = input[4_800..]
            .iter()
            .map(|&s| processor.process(s))
            .collect();
        let gain_db = 20.0 * (rms(&output) / rms(&input[4_800..])).log10();
        assert!(gain_db < -7.0, "-9 dB band only produced {gain_db:.1} dB");
    }

    #[test]
    fn preamp_scales_the_signal() {
        let eq = Equalizer {
            enabled: true,
            preamp_db: -6.0,
            ..Equalizer::default()
        };
        let mut processor = EqProcessor::new(48_000, 1, eq);
        let output: Vec<f32> = (0..64).map(|_| processor.process(0.5)).collect();
        let expected = 0.5 * 10f32.powf(-6.0 / 20.0);
        assert!((output[63] - expected).abs() < 1e-4);
    }

    #[test]
    fn output_never_clips_outside_the_sample_range() {
        let mut eq = Equalizer {
            enabled: true,
            preamp_db: 12.0,
            ..Equalizer::default()
        };
        eq.gains_db = [12.0; BAND_COUNT];
        let mut processor = EqProcessor::new(48_000, 2, eq);
        for _ in 0..10_000 {
            let out = processor.process(0.9);
            assert!(out.is_finite());
            assert!((-1.0..=1.0).contains(&out));
        }
    }

    #[test]
    fn channels_keep_separate_filter_state() {
        let mut eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        eq.gains_db[0] = 9.0;
        let mut processor = EqProcessor::new(48_000, 2, eq);

        // Left channel gets a hot signal, right is silent. If state leaked
        // between channels, the right output would not stay at zero.
        for _ in 0..1_000 {
            processor.process(0.5);
            assert_eq!(processor.process(0.0), 0.0);
        }
    }

    #[test]
    fn curve_reports_the_shape_of_the_filter() {
        let mut eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        eq.gains_db[5] = 9.0;
        let at_band = eq.response_db(1_000.0, 48_000.0);
        let far_away = eq.response_db(60.0, 48_000.0);
        assert!(at_band > 7.0, "band response {at_band}");
        assert!(far_away.abs() < 1.0, "distant response {far_away}");

        let bypassed = Equalizer::default();
        assert_eq!(bypassed.response_db(1_000.0, 48_000.0), 0.0);
    }

    #[test]
    fn presets_identify_themselves() {
        for preset in &PRESETS {
            let mut eq = Equalizer {
                enabled: true,
                ..Equalizer::default()
            };
            eq.apply_preset(preset);
            assert_eq!(eq.preset_name(), Some(preset.name));
        }
        let mut custom = Equalizer::default();
        custom.gains_db[3] = 1.0;
        assert_eq!(custom.preset_name(), None);
    }

    #[test]
    fn settings_are_clamped() {
        let eq = Equalizer {
            enabled: true,
            preamp_db: 99.0,
            gains_db: [99.0; BAND_COUNT],
        }
        .clamped();
        assert_eq!(eq.preamp_db, PREAMP_RANGE_DB);
        assert!(eq.gains_db.iter().all(|g| *g == GAIN_RANGE_DB));
    }

    #[test]
    fn shared_handle_updates_and_bumps_the_version() {
        let shared = EqShared::new(Equalizer::default());
        let before = shared.version();
        let mut eq = Equalizer {
            enabled: true,
            ..Equalizer::default()
        };
        eq.gains_db[2] = 4.0;
        shared.set(eq);
        assert_eq!(shared.snapshot().gains_db[2], 4.0);
        assert!(shared.version() > before);
    }
}
