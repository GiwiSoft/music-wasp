//! MilkDrop visualizer powered by `lactea` on the app's wgpu device.
//!
//! The engine renders into an offscreen `Rgba8Unorm` texture which is
//! registered with egui and drawn like any other image. Decoding, preset
//! expression evaluation and rendering all stay on the UI thread (lactea is
//! CPU-side for expressions and GPU-side for shaders); only the audio tap is
//! fed from the playback thread.

use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use egui::TextureId;
use lactea::{Engine, EngineConfig, FrameClock, Transition};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Largest internal render size. Fullscreen 4K panels are scaled down.
const MAX_WIDTH: u32 = 1280;
const MAX_HEIGHT: u32 = 720;

/// Largest PCM chunk pushed per UI frame, in interleaved samples.
const MAX_PCM_PER_FRAME: usize = 16_384;

pub struct Preset {
    pub name: String,
    pub bytes: Vec<u8>,
    /// `None` for bundled presets.
    pub path: Option<PathBuf>,
}

struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: TextureId,
}

pub struct Visualizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<egui::mutex::RwLock<eframe::egui_wgpu::Renderer>>,
    engine: Engine,
    target: Option<Target>,
    size: (u32, u32),
    presets: Vec<Preset>,
    current: usize,
    sample_rate: u32,
    started: Instant,
    frame: u32,
    last_switch: Instant,
    random_state: u64,
}

impl Visualizer {
    pub fn new(
        state: &RenderState,
        presets: Vec<Preset>,
        restore: Option<&str>,
    ) -> Result<Self, String> {
        if presets.is_empty() {
            return Err("no visualizer presets found".to_string());
        }
        let size = (640, 360);
        let mut engine = Engine::new(&state.device, &state.queue, engine_config(size, 44_100))
            .map_err(|error| format!("visualizer engine: {error}"))?;
        let index = restore
            .and_then(|name| presets.iter().position(|preset| preset.name == name))
            .unwrap_or(0);
        engine
            .load_preset(&presets[index].bytes, &presets[index].name, Transition::Cut)
            .map_err(|error| format!("preset '{}': {error}", presets[index].name))?;

        let mut visualizer = Self {
            device: state.device.clone(),
            queue: state.queue.clone(),
            renderer: Arc::clone(&state.renderer),
            engine,
            target: None,
            size: (0, 0),
            presets,
            current: index,
            sample_rate: 44_100,
            started: Instant::now(),
            frame: 0,
            last_switch: Instant::now(),
            random_state: seed(),
        };
        visualizer.ensure_target(size.0, size.1);
        Ok(visualizer)
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn current_name(&self) -> &str {
        self.presets
            .get(self.current)
            .map_or("", |preset| preset.name.as_str())
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Switch to a preset by list index. Errors leave the running preset alone.
    pub fn select(&mut self, index: usize, soft: bool) -> Result<(), String> {
        let Some(preset) = self.presets.get(index) else {
            return Err("preset index out of range".to_string());
        };
        let transition = if soft {
            Transition::Soft { seconds: 2.5 }
        } else {
            Transition::Cut
        };
        let name = preset.name.clone();
        self.engine
            .load_preset(&preset.bytes, &name, transition)
            .map_err(|error| format!("preset '{name}': {error}"))?;
        self.current = index;
        self.last_switch = Instant::now();
        Ok(())
    }

    pub fn select_by_name(&mut self, name: &str, soft: bool) -> Result<(), String> {
        let Some(index) = self.presets.iter().position(|preset| preset.name == name) else {
            return Err(format!("unknown preset '{name}'"));
        };
        self.select(index, soft)
    }

    /// Next preset, or a random one (never the same) when `shuffle` is true.
    pub fn next(&mut self, shuffle: bool) -> Result<(), String> {
        if self.presets.len() < 2 {
            return Ok(());
        }
        let index = if shuffle {
            self.random_state = self
                .random_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let offset = (self.random_state >> 33) as usize % (self.presets.len() - 1);
            (self.current + 1 + offset) % self.presets.len()
        } else {
            (self.current + 1) % self.presets.len()
        };
        self.select(index, true)
    }

    /// Advance automatically in shuffle mode. Returns true when it switched.
    pub fn maybe_advance(&mut self, shuffle: bool, interval_secs: u32) -> bool {
        if !shuffle || self.presets.len() < 2 {
            return false;
        }
        if self.last_switch.elapsed().as_secs() < u64::from(interval_secs.max(5)) {
            return false;
        }
        self.next(true).is_ok()
    }

    /// Recreate the engine when the track sample rate changes.
    pub fn ensure_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == 0 || sample_rate == self.sample_rate || self.size.0 == 0 {
            return;
        }
        let preset = self.current;
        let mut engine = match Engine::new(
            &self.device,
            &self.queue,
            engine_config(self.size, sample_rate),
        ) {
            Ok(engine) => engine,
            Err(error) => {
                eprintln!("visualizer engine for {sample_rate} Hz: {error}");
                return;
            }
        };
        let Some(entry) = self.presets.get(preset) else {
            return;
        };
        if let Err(error) = engine.load_preset(&entry.bytes, &entry.name, Transition::Cut) {
            eprintln!("visualizer preset reload: {error}");
            return;
        }
        self.engine = engine;
        self.sample_rate = sample_rate;
        self.started = Instant::now();
        self.frame = 0;
    }

    /// Render one frame into the offscreen texture and return its egui id.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        size: egui::Vec2,
        pcm: &[f32],
        channels: u16,
    ) -> Option<TextureId> {
        if size.x < 16.0 || size.y < 16.0 {
            return None;
        }
        let pixels_per_point = ctx.pixels_per_point();
        let (width, height) = fit(size, pixels_per_point);
        if !self.ensure_target(width, height) {
            return None;
        }
        if !pcm.is_empty() {
            self.engine.push_pcm_f32(
                &pcm[..pcm.len().min(MAX_PCM_PER_FRAME)],
                channels.max(1) as usize,
            );
        }
        let time = self.started.elapsed().as_secs_f64();
        let target = self.target.as_ref()?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("musicwasp-visualizer"),
            });
        self.engine.render(
            &mut encoder,
            &target.view,
            FrameClock {
                time,
                frame: self.frame,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        self.frame = self.frame.wrapping_add(1);
        Some(target.id)
    }

    fn ensure_target(&mut self, width: u32, height: u32) -> bool {
        if self.target.is_some() && self.size == (width, height) {
            return true;
        }
        if let Err(error) = self.engine.resize(width, height) {
            eprintln!("visualizer resize to {width}x{height}: {error}");
            return self.target.is_some();
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("musicwasp viz"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut renderer = self.renderer.write();
        let id = match self.target.take() {
            Some(old) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    &self.device,
                    &view,
                    wgpu::FilterMode::Linear,
                    old.id,
                );
                old.id
            }
            None => renderer.register_native_texture(&self.device, &view, wgpu::FilterMode::Linear),
        };
        drop(renderer);
        self.target = Some(Target {
            _texture: texture,
            view,
            id,
        });
        self.size = (width, height);
        true
    }
}

fn engine_config(size: (u32, u32), sample_rate: u32) -> EngineConfig {
    EngineConfig {
        width: size.0.max(1),
        height: size.1.max(1),
        output_format: wgpu::TextureFormat::Rgba8Unorm,
        sample_rate,
        auto_compat: true,
        ..Default::default()
    }
}

fn fit(size: egui::Vec2, pixels_per_point: f32) -> (u32, u32) {
    let mut width = (size.x * pixels_per_point).round().max(2.0) as u32;
    let mut height = (size.y * pixels_per_point).round().max(2.0) as u32;
    let scale = (MAX_WIDTH as f32 / width as f32)
        .min(MAX_HEIGHT as f32 / height as f32)
        .min(1.0);
    width = ((width as f32 * scale) as u32).max(2) & !1;
    height = ((height as f32 * scale) as u32).max(2) & !1;
    (width, height)
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |duration| duration.as_nanos() as u64)
        | 1
}

/// Bundled presets: six by M0Rf30 for lactea (MIT OR Apache-2.0) and the
/// original MilkDrop `Geiss` presets (Ryan Geiss, shipped with MilkDrop).
const BUNDLED: &[(&str, &[u8])] = &[
    (
        "Aurora Cathedral",
        include_bytes!("../../assets/presets/M0Rf30 - Aurora Cathedral.milk"),
    ),
    (
        "Bioluminescent Abyss",
        include_bytes!("../../assets/presets/M0Rf30 - Bioluminescent Abyss.milk"),
    ),
    (
        "Liquid Chrome Kaleidoscope",
        include_bytes!("../../assets/presets/M0Rf30 - Liquid Chrome Kaleidoscope.milk"),
    ),
    (
        "Nebula Forge",
        include_bytes!("../../assets/presets/M0Rf30 - Nebula Forge.milk"),
    ),
    (
        "Neon Sacred Geometry",
        include_bytes!("../../assets/presets/M0Rf30 - Neon Sacred Geometry.milk"),
    ),
    (
        "Tramonto sul Golfo",
        include_bytes!("../../assets/presets/M0Rf30 - Tramonto sul Golfo.milk"),
    ),
    (
        "Blur Mix 3",
        include_bytes!("../../assets/presets/Geiss - Blur Mix 3.milk"),
    ),
    (
        "Cauldron - painterly 5",
        include_bytes!("../../assets/presets/Geiss - Cauldron - painterly 5.milk"),
    ),
    (
        "Confetti",
        include_bytes!("../../assets/presets/Geiss - Confetti (Kaleidoscope Mix).milk"),
    ),
    (
        "Cosmic Dust 2",
        include_bytes!("../../assets/presets/Geiss - Cosmic Dust 2.milk"),
    ),
    (
        "Drop Shadow 1",
        include_bytes!("../../assets/presets/Geiss - Drop Shadow 1.milk"),
    ),
    (
        "Explosion 3",
        include_bytes!("../../assets/presets/Geiss - Explosion 3.milk"),
    ),
    (
        "Fog Tunnel",
        include_bytes!("../../assets/presets/Geiss - Fog Tunnel.milk"),
    ),
    (
        "Inkblot",
        include_bytes!("../../assets/presets/Geiss - Inkblot.milk"),
    ),
    (
        "Liquid Beats",
        include_bytes!(
            "../../assets/presets/Geiss - Liquid Beats (janky ripple warp reflecto).milk"
        ),
    ),
    (
        "Motion Blur 2 (Stahl RMX)",
        include_bytes!(
            "../../assets/presets/Geiss - Motion Blur 2 (Stahl's Neon Jelly 2 RMX).milk"
        ),
    ),
    (
        "Myriad Mosaics",
        include_bytes!("../../assets/presets/Geiss - Myriad Mosaics.milk"),
    ),
    (
        "Myriad Spirals",
        include_bytes!("../../assets/presets/Geiss - Myriad Spirals.milk"),
    ),
    (
        "Reaction Diffusion 2",
        include_bytes!("../../assets/presets/Geiss - Reaction Diffusion 2.milk"),
    ),
    (
        "Reaction Diffusion 3 (Lichen Mix)",
        include_bytes!("../../assets/presets/Geiss - Reaction Diffusion 3 (Lichen Mix).milk"),
    ),
    (
        "Reaction Diffusion 3 (Rippling Islands)",
        include_bytes!(
            "../../assets/presets/Geiss + Stahlregen - Reaction Diffusion 3 [Rippling Islands].milk"
        ),
    ),
    (
        "Reaction Diffusion 3 (Overload Mix 2)",
        include_bytes!(
            "../../assets/presets/Geiss and Zylot - Reaction Diffusion 3 (Overload Mix 2).milk"
        ),
    ),
    (
        "Skin Dots Multi-layer 3",
        include_bytes!("../../assets/presets/Geiss - Skin Dots Multi-layer 3.milk"),
    ),
    (
        "Spiral Artifact",
        include_bytes!("../../assets/presets/Geiss - Spiral Artifact.milk"),
    ),
    (
        "Swirlie 4",
        include_bytes!("../../assets/presets/Geiss - Swirlie 4.milk"),
    ),
    (
        "Tokamak Plus 4",
        include_bytes!("../../assets/presets/Geiss - Tokamak Plus 4.milk"),
    ),
];

/// Bundled presets plus `.milk` / `.milk2` files from `<data dir>/presets`.
pub fn load_presets() -> Vec<Preset> {
    let mut presets: Vec<Preset> = BUNDLED
        .iter()
        .map(|(name, bytes)| Preset {
            name: (*name).to_string(),
            bytes: bytes.to_vec(),
            path: None,
        })
        .collect();

    let Ok(data_dir) = musicwasp_core::db::data_dir() else {
        return presets;
    };
    let dir = data_dir.join("presets");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return presets;
    };
    let mut user = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_preset = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("milk") || extension.eq_ignore_ascii_case("milk2")
            });
        if !is_preset {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Preset")
            .to_string();
        if presets.iter().any(|preset| preset.name == name) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            user.push(Preset {
                name,
                bytes,
                path: Some(path),
            });
        }
    }
    user.sort_by(|left, right| left.name.cmp(&right.name));
    presets.extend(user);
    presets
}

#[cfg(all(test, target_os = "linux"))]
mod gpu_tests {
    use super::*;

    #[test]
    fn every_bundled_preset_loads() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("wgpu adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("wgpu device");
        let mut engine = Engine::new(&device, &queue, engine_config((320, 180), 44_100))
            .expect("visualizer engine");
        let presets = load_presets();
        assert!(presets.len() >= BUNDLED.len());
        for preset in presets.iter().take(BUNDLED.len()) {
            engine
                .load_preset(&preset.bytes, &preset.name, Transition::Cut)
                .unwrap_or_else(|error| panic!("{}: {error}", preset.name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_presets_are_embedded() {
        let presets = load_presets();
        assert!(presets.len() >= BUNDLED.len());
        for (name, _) in BUNDLED {
            assert!(
                presets.iter().any(|preset| preset.name == *name),
                "missing {name}"
            );
        }
        assert!(presets[0].bytes.starts_with(b"//") || presets[0].bytes.len() > 100);
    }

    #[test]
    fn render_size_is_capped_and_even() {
        assert_eq!(fit(egui::vec2(320.0, 180.0), 1.0), (320, 180));
        assert_eq!(fit(egui::vec2(1920.0, 1080.0), 2.0), (1280, 720));
        let (width, height) = fit(egui::vec2(1920.0, 1080.0), 1.0);
        assert_eq!((width % 2, height % 2), (0, 0));
    }
}
