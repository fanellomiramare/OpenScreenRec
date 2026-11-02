//! Audio capture for OpenScreenRec: microphone and system loopback.
//!
//! Backend plan (lands in M2):
//!
//! | Source  | API per OS                                                                     |
//! | ------- | ------------------------------------------------------------------------------ |
//! | Mic     | `cpal` on every platform                                                       |
//! | System  | WASAPI loopback (Windows) · ScreenCaptureKit audio (macOS) · Pulse/PipeWire monitor (Linux) |
//!
//! All chunks carry timestamps on the same monotonic clock as video
//! frames, which keeps A/V sync and pause handling simple.

/// Which audio source a stream captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Microphone,
    /// Everything the user hears (loopback / desktop audio).
    System,
}

/// One buffer of interleaved f32 samples.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Capture timestamp in microseconds on the pipeline's monotonic clock.
    pub pts_us: u64,
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved f32 samples, `samples.len() == frames * channels`.
    pub samples: Vec<f32>,
}

impl AudioChunk {
    /// Duration of the chunk in seconds.
    pub fn duration_secs(&self) -> f64 {
        let channels = u32::from(self.channels.max(1));
        self.samples.len() as f64 / (f64::from(self.sample_rate) * f64::from(channels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_of_one_frame() {
        let chunk = AudioChunk {
            pts_us: 0,
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0; 2],
        };
        assert!((chunk.duration_secs() - 1.0 / 48_000.0).abs() < 1e-12);
    }
}
