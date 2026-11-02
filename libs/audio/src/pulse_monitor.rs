//! System audio capture on Linux: the PulseAudio monitor of the
//! default sink (`@DEFAULT_MONITOR@`), via `libpulse-simple`. Under
//! PipeWire desktops the same monitor source is provided by
//! `pipewire-pulse`, so this covers both worlds.
//!
//! The stream reads continuously (monitors deliver silence when
//! nothing plays), so a plain stop-flag between reads is enough — no
//! blocking teardown races. Runs on its own thread, same drop-on-
//! overflow policy as the other backends.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use libpulse_binding::sample::{Format, Spec};
use libpulse_simple_binding::Simple;

use crate::{AudioChunk, AudioConfig, AudioError, AudioSource, SourceKind};
use osr_common::monotonic_us;

const CHANNEL_CAPACITY: usize = 64;
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Bytes per read: ~85ms of 48kHz stereo f32 — enough to amortize
/// calls, small enough to keep the stop latency negligible.
const READ_BYTES: usize = 32_768;

/// Capture of the default sink's monitor.
pub struct PulseMonitorSource {
    rx: Receiver<AudioChunk>,
    thread: Option<JoinHandle<()>>,
    stop: std::sync::Arc<AtomicBool>,
    config: AudioConfig,
}

impl PulseMonitorSource {
    pub fn new() -> Result<Self, AudioError> {
        let spec = Spec {
            format: Format::F32le,
            channels: CHANNELS,
            rate: SAMPLE_RATE,
        };
        if !spec.is_valid() {
            return Err(AudioError::Device("invalid audio spec".into()));
        }

        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);

        let thread = std::thread::Builder::new()
            .name("openscreenrec-system-audio".into())
            .spawn(move || {
                if let Err(message) = capture_loop(&thread_stop, tx) {
                    tracing::warn!("system audio capture stopped: {message}");
                }
            })
            .map_err(|e| AudioError::Device(format!("failed to spawn the audio thread: {e}")))?;

        Ok(Self {
            rx,
            thread: Some(thread),
            stop,
            config: AudioConfig {
                sample_rate: SAMPLE_RATE,
                channels: CHANNELS,
            },
        })
    }
}

/// Opens the monitor stream and pumps chunks until `stop` is set. The
/// stream is opened (and closed) on this thread only.
fn capture_loop(stop: &AtomicBool, tx: SyncSender<AudioChunk>) -> Result<(), String> {
    let spec = Spec {
        format: Format::F32le,
        channels: CHANNELS,
        rate: SAMPLE_RATE,
    };
    let simple = Simple::new(
        None,                  // default server
        "OpenScreenRec",       // application name
        libpulse_binding::stream::Direction::Record,
        Some("@DEFAULT_MONITOR@"), // everything the user hears
        "screen recording",    // stream description
        &spec,
        None,                  // channel map
        None,                  // buffer attributes
    )
    .map_err(|e| {
        AudioError::Device(format!(
            "PulseAudio monitor unavailable ({e}) — is PulseAudio/PipeWire's pulse replacement running?"
        ))
    })?;

    let mut buffer = vec![0u8; READ_BYTES];
    while !stop.load(Ordering::Relaxed) {
        simple
            .read(&mut buffer)
            .map_err(|e| AudioError::Stream(format!("monitor read failed: {e}")))?;
        let mut samples = Vec::with_capacity(buffer.len() / 4);
        for chunk in buffer.as_chunks::<4>().0 {
            samples.push(f32::from_le_bytes(*chunk));
        }
        let audio_chunk = AudioChunk {
            kind: SourceKind::System,
            pts_us: monotonic_us(),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            samples,
        };
        let _ = tx.try_send(audio_chunk);
    }
    Ok(())
}

impl AudioSource for PulseMonitorSource {
    fn start(&mut self) -> Result<(), AudioError> {
        Ok(()) // the stream starts in the constructor; see the trait docs.
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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

impl Drop for PulseMonitorSource {
    fn drop(&mut self) {
        self.stop();
    }
}
