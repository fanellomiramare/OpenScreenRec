//! System audio capture on macOS: ScreenCaptureKit audio output, via
//! the [`screencapturekit`](https://github.com/svtslf/screencapturekit)
//! crate. A dedicated SCK stream is opened with `captures_audio(true)`;
//! video samples from that stream are ignored.
//!
//! SCK delivers interleaved 32-bit float samples at the configured
//! rate and channel count; we read the sample buffer's block buffer
//! and reinterpret the bytes as f32. Requires the Screen Recording
//! permission, same as video (see `osr_capture::backend`).

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use screencapturekit::prelude::*;

use crate::{AudioChunk, AudioConfig, AudioError, AudioSource, SourceKind};
use osr_common::monotonic_us;

const CHANNEL_CAPACITY: usize = 64;
const SAMPLE_RATE: i32 = 48_000;
const CHANNELS: i32 = 2;

struct SckAudioHandler {
    tx: Arc<Mutex<SyncSender<AudioChunk>>>,
}

impl SCStreamOutputTrait for SckAudioHandler {
    fn did_output_sample_buffer(&mut self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if of_type != SCStreamOutputType::Audio {
            return;
        }
        use screencapturekit::cm::sample_buffer::CMSampleBufferDataBufferExt;
        let Some(block) = sample.data_buffer_local() else {
            return;
        };
        let length = block.data_length();
        let Some((pointer, readable)) = block.data_pointer(0) else {
            return;
        };
        let bytes = unsafe { std::slice::from_raw_parts(pointer, readable.min(length)) };
        let mut samples = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.as_chunks::<4>().0 {
            samples.push(f32::from_le_bytes(*chunk));
        }
        if samples.is_empty() {
            return;
        }
        let chunk = AudioChunk {
            kind: SourceKind::System,
            pts_us: monotonic_us(),
            sample_rate: SAMPLE_RATE as u32,
            channels: CHANNELS as u16,
            samples,
        };
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.try_send(chunk);
        }
    }
}

/// Capture of system audio on macOS through a dedicated SCK stream.
pub struct SckSystemAudioSource {
    rx: Receiver<AudioChunk>,
    stream: SCStream,
    config: AudioConfig,
}

impl SckSystemAudioSource {
    pub fn new() -> Result<Self, AudioError> {
        let content = SCShareableContent::get().map_err(|e| {
            AudioError::Device(format!(
                "screen content unavailable — grant Screen Recording permission ({e})"
            ))
        })?;
        let Some(display) = content.displays().into_iter().next() else {
            return Err(AudioError::Device("no display found".into()));
        };
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build()
            .map_err(|e| AudioError::Device(format!("content filter failed: {e}")))?;
        let config = SCStreamConfiguration::new()
            .with_width(2)
            .with_height(2)
            .with_captures_audio(true)
            .with_sample_rate(SAMPLE_RATE)
            .with_channel_count(CHANNELS);

        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let mut stream = SCStream::new(&filter, &config)
            .map_err(|e| AudioError::Device(format!("failed to create the SCK stream: {e}")))?;
        stream
            .add_output_handler(
                SckAudioHandler {
                    tx: Arc::new(Mutex::new(tx)),
                },
                SCStreamOutputType::Audio,
            )
            .map_err(|e| AudioError::Device(format!("failed to attach the handler: {e}")))?;
        stream
            .start_capture()
            .map_err(|e| AudioError::Device(format!("failed to start capture: {e}")))?;

        Ok(Self {
            rx,
            stream,
            config: AudioConfig {
                sample_rate: SAMPLE_RATE as u32,
                channels: CHANNELS as u16,
            },
        })
    }
}

impl AudioSource for SckSystemAudioSource {
    fn start(&mut self) -> Result<(), AudioError> {
        Ok(()) // the SCK stream starts in the constructor; see the trait docs.
    }

    fn stop(&mut self) {
        let _ = self.stream.stop_capture();
    }

    fn next_chunk(&mut self, timeout: Duration) -> Result<Option<AudioChunk>, AudioError> {
        match self.rx.recv_timeout(timeout) {
            Ok(chunk) => Ok(Some(chunk)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(AudioError::Stream(
                "the system audio stream closed unexpectedly".into(),
            )),
        }
    }

    fn config(&self) -> AudioConfig {
        self.config
    }
}

impl Drop for SckSystemAudioSource {
    fn drop(&mut self) {
        self.stop();
    }
}
