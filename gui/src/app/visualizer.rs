//! MilkDrop visualizer lifecycle and preset selection.

use super::*;

impl App {
    // ---- visualizer ----

    /// Create the MilkDrop engine on first use.
    pub fn ensure_visualizer(&mut self) -> bool {
        if self.visualizer.is_some() {
            return true;
        }
        let Some(state) = self.wgpu_state.clone() else {
            self.viz_error = Some("the wgpu renderer is not available".to_string());
            return false;
        };
        let presets = crate::visualizer::load_presets();
        match crate::visualizer::Visualizer::new(
            &state,
            presets,
            Some(self.settings.visualizer.preset.as_str()),
        ) {
            Ok(visualizer) => {
                self.visualizer = Some(visualizer);
                self.viz_error = None;
                true
            }
            Err(error) => {
                self.viz_error = Some(error);
                false
            }
        }
    }

    pub fn set_visualizer_enabled(&mut self, enabled: bool, ctx: &egui::Context) {
        self.settings.visualizer.enabled = enabled;
        if enabled {
            self.ensure_visualizer();
        } else {
            self.viz_fullscreen = false;
            self.viz_pcm.clear();
        }
        self.persist_settings();
        ctx.request_repaint();
    }

    /// Drain the audio tap and advance shuffle mode. Call once per frame before
    /// drawing the UI.
    pub fn update_visualizer(&mut self, ctx: &egui::Context) {
        if !self.settings.visualizer.enabled {
            self.viz_pcm.clear();
            return;
        }
        if let Some(player) = &self.player {
            let tap = player.audio_tap();
            let (sample_rate, channels) = tap.format();
            self.viz_channels = channels;
            if let Some(visualizer) = &mut self.visualizer {
                visualizer.ensure_sample_rate(sample_rate);
            }
            self.viz_pcm.clear();
            tap.drain_into(&mut self.viz_pcm, 16_384);
        } else {
            self.viz_pcm.clear();
        }

        let shuffle = self.settings.visualizer.mode == crate::settings::VisualizerMode::Shuffle;
        let interval = self.settings.visualizer.interval_secs;
        let switched = self.visualizer.as_mut().and_then(|visualizer| {
            visualizer
                .maybe_advance(shuffle, interval)
                .then(|| visualizer.current_name().to_string())
        });
        if let Some(name) = switched {
            self.settings.visualizer.preset = name;
            self.persist_settings();
        }

        let fps = self.settings.visualizer.fps.clamp(15, 60);
        ctx.request_repaint_after(std::time::Duration::from_millis(1000 / u64::from(fps)));
    }

    /// Render the visualizer for `size` and return the egui texture. Rendering
    /// happens at most once per frame; repeated calls reuse the same texture.
    pub fn render_visualizer(
        &mut self,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> Option<egui::TextureId> {
        if !self.settings.visualizer.enabled {
            return None;
        }
        if self.visualizer.is_none() && !self.ensure_visualizer() {
            return None;
        }
        let mut pcm = std::mem::take(&mut self.viz_pcm);
        let result = self
            .visualizer
            .as_mut()
            .and_then(|visualizer| visualizer.render(ctx, size, &pcm, self.viz_channels));
        pcm.clear();
        self.viz_pcm = pcm;
        result
    }

    pub fn select_visualizer_preset(&mut self, name: &str) {
        self.settings.visualizer.preset = name.to_string();
        if let Some(visualizer) = &mut self.visualizer {
            match visualizer.select_by_name(name, true) {
                Ok(()) => self.viz_error = None,
                Err(error) => self.viz_error = Some(error),
            }
        }
        self.persist_settings();
    }

    pub fn next_visualizer_preset(&mut self) {
        let shuffle = self.settings.visualizer.mode == crate::settings::VisualizerMode::Shuffle;
        if let Some(visualizer) = &mut self.visualizer {
            match visualizer.next(shuffle) {
                Ok(()) => {
                    self.settings.visualizer.preset = visualizer.current_name().to_string();
                    self.viz_error = None;
                }
                Err(error) => self.viz_error = Some(error),
            }
        }
        self.persist_settings();
    }

    pub fn previous_visualizer_preset(&mut self) {
        if let Some(visualizer) = &mut self.visualizer {
            let count = visualizer.presets().len();
            if count == 0 {
                return;
            }
            let index = if visualizer.current_index() == 0 {
                count - 1
            } else {
                visualizer.current_index() - 1
            };
            match visualizer.select(index, true) {
                Ok(()) => {
                    self.settings.visualizer.preset = visualizer.current_name().to_string();
                    self.viz_error = None;
                }
                Err(error) => self.viz_error = Some(error),
            }
        }
        self.persist_settings();
    }

    pub fn set_visualizer_mode(&mut self, mode: crate::settings::VisualizerMode) {
        self.settings.visualizer.mode = mode;
        self.persist_settings();
    }
}
