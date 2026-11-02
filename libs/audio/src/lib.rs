//! Audio capture for OpenScreenRec: microphone and system loopback.
//!
//! Backends:
//!
//! | Source  | API per OS                                                                        |
//! | ------- | --------------------------------------------------------------------------------- |
//! | Mic     | `cpal` on every platform                                                          |
//! | System  | WASAPI loopback via `wasapi` (Windows) · ScreenCaptureKit audio (macOS, M4) · Pulse/PipeWire monitor (Linux, M5) |
//!
//! Chunks are interleaved f32 and carry timestamps on the same
//! monotonic clock as video frames. During recording they are written
//! to temporary WAV files and muxed with the video at the end (M6
//! replaces that with the pure-Rust muxer).

pub mod cpal_source;
pub mod wav;

pub use wav::WavAudioWriter;

#[cfg(target_os = "windows")]
pub mod wasapi_loopback;

use std::time::Duration;

/// Which audio source a stream captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Microphone,
    /// Everything the user hears (loopback / desktop audio).
    System,
}

/// Capture format reported by a source before (and while) it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

/// One buffer of interleaved f32 samples.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Which source the chunk came from.
    pub kind: SourceKind,
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

/// Why starting or running an audio capture failed.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// This OS doesn't provide the requested source (yet).
    #[error("audio source not implemented on this platform: {0}")]
    Unsupported(String),
    /// No usable device, or the audio service refused us.
    #[error("audio device error: {0}")]
    Device(String),
    /// The stream stopped unexpectedly.
    #[error("audio stream failed: {0}")]
    Stream(String),
    /// Writing the temporary WAV file failed.
    #[error("wav writing failed: {0}")]
    Wav(#[from] hound::Error),
    /// Local file system access failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// A source of audio chunks: one running capture stream.
///
/// Same contract as [`osr_capture`'s `FrameSource`](https://docs.rs/osr-capture):
/// `start` → `next_chunk` loop → `stop`. Sources may already be live
/// when constructed; the pipeline drains them with
/// `next_chunk(Duration::ZERO)` between video frames.
pub trait AudioSource: Send {
    /// Begin delivering chunks. Must be called before [`Self::next_chunk`].
    fn start(&mut self) -> Result<(), AudioError>;

    /// Stop the stream and release OS resources.
    fn stop(&mut self);

    /// Block up to `timeout` for the next chunk; `Ok(None)` when none
    /// arrived in time. `Duration::ZERO` acts as a non-blocking drain.
    fn next_chunk(&mut self, timeout: Duration) -> Result<Option<AudioChunk>, AudioError>;

    /// Capture format (known as soon as the source is constructed).
    fn config(&self) -> AudioConfig;
}

/// Create a capture source for the default microphone.
pub fn create_microphone_source() -> Result<Box<dyn AudioSource>, AudioError> {
    Ok(Box::new(cpal_source::CpalSource::default_input()?))
}

/// Create a capture source for everything the user hears.
#[cfg(target_os = "windows")]
pub fn create_system_audio_source() -> Result<Box<dyn AudioSource>, AudioError> {
    Ok(Box::new(wasapi_loopback::WasapiLoopbackSource::new()?))
}

/// Create a capture source for everything the user hears.
///
/// System audio lands on macOS in M4 and on Linux in M5.
#[cfg(not(target_os = "windows"))]
pub fn create_system_audio_source() -> Result<Box<dyn AudioSource>, AudioError> {
    Err(AudioError::Unsupported(
        "system audio lands on macOS in M4 and on Linux in M5".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_of_one_frame() {
        let chunk = AudioChunk {
            kind: SourceKind::Microphone,
            pts_us: 0,
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0; 2],
        };
        assert!((chunk.duration_secs() - 1.0 / 48_000.0).abs() < 1e-12);
    }
}
