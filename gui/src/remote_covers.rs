//! Bounded cache for remote podcast and radio artwork.
//!
//! Downloads and decoding run on worker threads. The UI only requests a URL and
//! uploads finished textures on the next frame, like the album cover cache.

use egui::{ColorImage, TextureHandle};
use musicwasp_core::netguard::Access;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};

const CAPACITY: usize = 128;
const WORKERS: usize = 2;
const PENDING_LIMIT: usize = 32;
const SIDE: u32 = 256;

struct Job {
    url: String,
    access: Access,
}

#[derive(Default)]
pub(crate) struct RemoteCoverCache {
    entries: HashMap<String, Option<TextureHandle>>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<(String, Option<ColorImage>)>>,
}

impl RemoteCoverCache {
    /// Upload finished artwork. Call once per frame.
    pub(crate) fn pump(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        if let Some(done) = &self.done {
            while let Ok(item) = done.try_recv() {
                finished.push(item);
            }
        }
        for (url, image) in finished {
            if !self.pending.remove(&url) {
                continue;
            }
            let texture = image.map(|image| {
                ctx.load_texture(
                    format!("remote-cover-{url}"),
                    image,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.insert(url, texture);
        }
    }

    /// Cached texture for `url`, or `None` while the first request runs.
    pub(crate) fn texture(
        &mut self,
        ctx: &egui::Context,
        url: &str,
        access: Access,
    ) -> Option<TextureHandle> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        if let Some(value) = self.entries.get(url).cloned() {
            self.touch(url);
            return value;
        }
        self.request(ctx, url, access);
        None
    }

    fn request(&mut self, ctx: &egui::Context, url: &str, access: Access) {
        if self.pending.contains(url) || self.pending.len() >= PENDING_LIMIT {
            return;
        }
        if self.jobs.is_none() {
            let (job_tx, job_rx) = mpsc::channel::<Job>();
            let (done_tx, done_rx) = mpsc::channel();
            let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
            for _ in 0..WORKERS {
                let job_rx = std::sync::Arc::clone(&job_rx);
                let done_tx = done_tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || loop {
                    let job = {
                        let guard = job_rx.lock().unwrap_or_else(|error| error.into_inner());
                        guard.recv()
                    };
                    let Ok(job) = job else {
                        break;
                    };
                    let image = musicwasp_core::artwork::fetch(&job.url, job.access)
                        .ok()
                        .and_then(|bytes| crate::images::from_bytes(&bytes).ok())
                        .map(|image| {
                            let rgba = trim_background(image.to_rgba8());
                            let resized = image::DynamicImage::ImageRgba8(rgba).resize(
                                SIDE,
                                SIDE,
                                image::imageops::FilterType::Triangle,
                            );
                            // Letterbox instead of cropping: station logos are often
                            // wider than they are tall.
                            let mut canvas =
                                image::RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([0, 0, 0, 0]));
                            let rgba = resized.to_rgba8();
                            let x = i64::from((SIDE - rgba.width()) / 2);
                            let y = i64::from((SIDE - rgba.height()) / 2);
                            image::imageops::overlay(&mut canvas, &rgba, x, y);
                            ColorImage::from_rgba_unmultiplied(
                                [SIDE as usize, SIDE as usize],
                                &canvas.into_raw(),
                            )
                        });
                    if done_tx.send((job.url, image)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                });
            }
            self.jobs = Some(job_tx);
            self.done = Some(done_rx);
        }
        let sent = self.jobs.as_ref().is_some_and(|jobs| {
            jobs.send(Job {
                url: url.to_string(),
                access,
            })
            .is_ok()
        });
        if sent {
            self.pending.insert(url.to_string());
        }
    }

    fn insert(&mut self, url: String, value: Option<TextureHandle>) {
        if self.entries.insert(url.clone(), value).is_none() {
            self.order.push_back(url);
        } else {
            self.touch(&url);
        }
        while self.entries.len() > CAPACITY {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn touch(&mut self, url: &str) {
        if let Some(index) = self.order.iter().position(|item| item == url) {
            if let Some(item) = self.order.remove(index) {
                self.order.push_back(item);
            }
        }
    }
}

/// Drop uniform margins so a logo on a plain background fills its tile.
fn trim_background(image: image::RgbaImage) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return image;
    }
    let background = image.get_pixel(0, 0).0;
    let differs = |pixel: &image::Rgba<u8>| {
        pixel
            .0
            .iter()
            .zip(background.iter())
            .any(|(value, base)| value.abs_diff(*base) > 12)
    };
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if differs(pixel) {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    if left > right || top > bottom {
        return image;
    }
    image::imageops::crop_imm(&image, left, top, right - left + 1, bottom - top + 1).to_image()
}
