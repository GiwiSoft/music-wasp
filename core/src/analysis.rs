//! Lock-free audio tap for visualizers.
//!
//! [`DspSource`](crate::dsp::DspSource) pushes post-EQ samples into an
//! [`AudioTap`] without allocating or locking. The GUI drains a bounded number
//! of samples each frame and feeds its visualizer. When the consumer falls
//! behind, old samples are dropped instead of blocking the audio callback.

use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::Arc;

/// About 1.5 seconds of interleaved stereo at 44.1 kHz.
pub const TAP_CAPACITY: usize = 131_072;

pub struct AudioTap {
    queue: ArrayQueue<f32>,
    sample_rate: AtomicU32,
    channels: AtomicU16,
}

impl AudioTap {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: ArrayQueue::new(TAP_CAPACITY),
            sample_rate: AtomicU32::new(44_100),
            channels: AtomicU16::new(2),
        })
    }

    /// Record the format of the source currently feeding the tap.
    pub fn set_format(&self, sample_rate: u32, channels: u16) {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.channels.store(channels.max(1), Ordering::Relaxed);
    }

    /// `(sample_rate, channels)` of the current stream.
    pub fn format(&self) -> (u32, u16) {
        (
            self.sample_rate.load(Ordering::Relaxed),
            self.channels.load(Ordering::Relaxed),
        )
    }

    /// Push one interleaved sample. Never blocks; drops the sample when full.
    #[inline]
    pub fn push(&self, sample: f32) {
        let _ = self.queue.push(sample);
    }

    /// Drop everything queued, for example when a new track starts.
    pub fn clear(&self) {
        while self.queue.pop().is_some() {}
    }

    /// Move up to `max` samples into `out`. Returns the number appended.
    pub fn drain_into(&self, out: &mut Vec<f32>, max: usize) -> usize {
        let start = out.len();
        while out.len() - start < max {
            match self.queue.pop() {
                Some(sample) => out.push(sample),
                None => break,
            }
        }
        out.len() - start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_round_trips_and_drops_when_full() {
        let tap = AudioTap::new();
        tap.set_format(48_000, 2);
        assert_eq!(tap.format(), (48_000, 2));
        for value in 0..8 {
            tap.push(value as f32);
        }
        let mut out = Vec::new();
        assert_eq!(tap.drain_into(&mut out, 4), 4);
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0]);
        tap.clear();
        out.clear();
        assert_eq!(tap.drain_into(&mut out, 4), 0);
    }
}
