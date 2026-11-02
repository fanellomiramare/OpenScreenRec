//! Microphone capture via [`cpal`] — works on every platform.
//!
//! The default input device is captured in its native shared-mode
//! format; samples are converted to f32 in the device callback and
//! shipped to the pipeline through a bounded channel (overflow frames
//! are dropped, same back-pressure policy as the video backends).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{AudioChunk, AudioConfig, AudioError, AudioSource, SourceKind};
use osr_common::monotonic_us;

/// Chunks buffered between the cpal callback thread and the consumer.
const CHANNEL_CAPACITY: usize = 64;

/// Capture of the default input device (microphone).
pub struct CpalSource {
    rx: Receiver<AudioChunk>,
    stream: Option<cpal::Stream>,
    config: AudioConfig,
}

impl CpalSource {
    /// Open the host's default input device.
    pub fn default_input() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| AudioError::Device("no microphone found".into()))?;

        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::Device(format!("microphone config unavailable: {e}")))?;
        let stream_config: cpal::StreamConfig = supported.into();
        let config = AudioConfig {
            sample_rate: stream_config.sample_rate,
            channels: stream_config.channels,
        };

        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let halted = Arc::new(AtomicBool::new(false));

        macro_rules! build_stream {
            ($sample:ty) => {{
                let tx = tx.clone();
                let halted = Arc::clone(&halted);
                device
                    .build_input_stream(
                        stream_config.clone(),
                        move |data: &[$sample], _info: &cpal::InputCallbackInfo| {
                            if halted.load(std::sync::atomic::Ordering::Relaxed) {
                                return;
                            }
                            let mut samples = Vec::with_capacity(data.len());
                            samples.extend(data.iter().map(|s| s.to_sample::<f32>()));
                            let chunk = AudioChunk {
                                kind: SourceKind::Microphone,
                                pts_us: monotonic_us(),
                                sample_rate: config.sample_rate,
                                channels: config.channels,
                                samples,
                            };
                            let _ = tx.try_send(chunk);
                        },
                        |err| tracing::warn!("microphone stream error: {err}"),
                        None,
                    )
                    .map_err(|e| {
                        AudioError::Device(format!("failed to open the microphone: {e}"))
                    })?
            }};
        }

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build_stream!(f32),
            cpal::SampleFormat::I16 => build_stream!(i16),
            cpal::SampleFormat::U16 => build_stream!(u16),
            other => {
                return Err(AudioError::Device(format!(
                    "unsupported microphone sample format: {other:?}"
                )));
            }
        };

        Ok(Self {
            rx,
            stream: Some(stream),
            config,
        })
    }
}

impl AudioSource for CpalSource {
    fn start(&mut self) -> Result<(), AudioError> {
        // cpal streams start playing on construction; keep the explicit
        // call to stay symmetric with the other backends.
        if let Some(stream) = self.stream.as_ref() {
            stream
                .play()
                .map_err(|e| AudioError::Device(format!("failed to start the microphone: {e}")))?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        // Dropping the stream stops capture and joins its thread.
        self.stream = None;
    }

    fn next_chunk(&mut self, timeout: Duration) -> Result<Option<AudioChunk>, AudioError> {
        match self.rx.recv_timeout(timeout) {
            Ok(chunk) => Ok(Some(chunk)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(AudioError::Stream(
                "the microphone stream closed unexpectedly".into(),
            )),
        }
    }

    fn config(&self) -> AudioConfig {
        self.config
    }
}

impl Drop for CpalSource {
    fn drop(&mut self) {
        self.stop();
    }
}
