//! System audio capture on Windows: WASAPI loopback, via the
//! [`wasapi`](https://github.com/HEnquist/wasapi-rs) crate (MIT).
//!
//! Loopback capture opens the default *render* endpoint (the speakers)
//! and initializes its client for capture — WASAPI then delivers
//! everything the user hears, in the native float mix format.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode};

use crate::{AudioChunk, AudioConfig, AudioError, AudioSource, SourceKind};
use osr_common::monotonic_us;

/// Chunks buffered between the capture thread and the consumer.
const CHANNEL_CAPACITY: usize = 64;
/// How long the constructor waits for the capture thread to report its
/// format before giving up (the audio service may be busy or absent).
const INIT_TIMEOUT: Duration = Duration::from_secs(3);
/// Poll cap while waiting for loopback events; also bounds stop latency.
const EVENT_TIMEOUT_MS: u32 = 100;

/// Capture of the default render endpoint (system audio).
pub struct WasapiLoopbackSource {
    rx: Receiver<AudioChunk>,
    thread: Option<JoinHandle<()>>,
    stop: std::sync::Arc<AtomicBool>,
    config: AudioConfig,
}

impl WasapiLoopbackSource {
    pub fn new() -> Result<Self, AudioError> {
        // COM initialization is per-thread, so the whole WASAPI setup
        // lives on the dedicated capture thread; it reports the mix
        // format (or the failure) back through `init_rx`.
        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let (init_tx, init_rx) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);

        let thread = std::thread::Builder::new()
            .name("openscreenrec-system-audio".into())
            .spawn(move || {
                // The capture thread reports its format through `init_tx`
                // as soon as setup succeeds; if it fails before that, the
                // error is forwarded here.
                if let Err(e) = capture_loop(&thread_stop, tx, init_tx.clone()) {
                    let _ = init_tx.send(Err(e.to_string()));
                }
            })
            .map_err(|e| AudioError::Device(format!("failed to spawn the audio thread: {e}")))?;

        let config = init_rx
            .recv_timeout(INIT_TIMEOUT)
            .map_err(|_| AudioError::Device("system audio did not initialize in time".into()))?
            .map_err(AudioError::Device)?;

        Ok(Self {
            rx,
            thread: Some(thread),
            stop,
            config,
        })
    }
}

/// Runs on the capture thread: set up the loopback client, report the
/// format through `init_tx`, then pump chunks until `stop` is set or
/// the stream fails.
fn capture_loop(
    stop: &AtomicBool,
    tx: SyncSender<AudioChunk>,
    init_tx: std::sync::mpsc::Sender<Result<AudioConfig, String>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    wasapi::initialize_mta()
        .ok()
        .map_err(|_| AudioError::Device("failed to initialize COM for WASAPI".to_string()))?;

    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| AudioError::Device(format!("no default audio output found: {e}")))?;
    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|e| AudioError::Device(format!("failed to open the output device: {e}")))?;

    let format = audio_client
        .get_mixformat()
        .map_err(|e| AudioError::Device(format!("mix format unavailable: {e}")))?;
    match format.get_subformat() {
        Ok(SampleType::Float) => {}
        other => {
            return Err(AudioError::Device(format!(
                "unexpected system audio sample format: {other:?}"
            ))
            .into());
        }
    }
    let config = AudioConfig {
        sample_rate: format.get_samplespersec(),
        channels: format.get_nchannels(),
    };

    let (default_period, _min_period) = audio_client
        .get_device_period()
        .map_err(|e| AudioError::Device(format!("device period unavailable: {e}")))?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: default_period,
    };
    // Render endpoint + capture direction = loopback capture.
    audio_client
        .initialize_client(&format, &Direction::Capture, &mode)
        .map_err(|e| AudioError::Device(format!("failed to initialize loopback: {e}")))?;

    let event = audio_client
        .set_get_eventhandle()
        .map_err(|e| AudioError::Device(format!("event handle unavailable: {e}")))?;
    let buffer_frame_count = audio_client
        .get_buffer_size()
        .map_err(|e| AudioError::Device(format!("buffer size unavailable: {e}")))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| AudioError::Device(format!("capture client unavailable: {e}")))?;
    audio_client
        .start_stream()
        .map_err(|e| AudioError::Device(format!("failed to start the stream: {e}")))?;

    // Keep-alive: an idle render endpoint is suspended by Windows and
    // loopback capture then receives nothing at all — not even silence.
    // A silent render stream on the same endpoint keeps it running, so
    // loopback packets flow continuously. Polling mode: we top it up
    // from the capture loop, no event handle needed.
    let mut keepalive_client = device
        .get_iaudioclient()
        .map_err(|e| AudioError::Device(format!("failed to open the keep-alive stream: {e}")))?;
    let keepalive_mode = StreamMode::PollingShared {
        autoconvert: true,
        buffer_duration_hns: default_period,
    };
    keepalive_client
        .initialize_client(&format, &Direction::Render, &keepalive_mode)
        .map_err(|e| {
            AudioError::Device(format!("failed to initialize the keep-alive stream: {e}"))
        })?;
    let keepalive_render = keepalive_client
        .get_audiorenderclient()
        .map_err(|e| AudioError::Device(format!("keep-alive render client unavailable: {e}")))?;
    keepalive_client
        .start_stream()
        .map_err(|e| AudioError::Device(format!("failed to start the keep-alive stream: {e}")))?;

    let block_align = format.get_blockalign() as usize;
    let mut buffer = vec![0u8; buffer_frame_count as usize * block_align];
    let silence = vec![0u8; buffer_frame_count as usize * block_align];

    let _ = init_tx.send(Ok(config));

    while !stop.load(Ordering::Relaxed) {
        // Timeouts let the thread notice `stop` even in silence.
        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            continue;
        }
        let (frames_read, _info) = match capture_client.read_from_device(&mut buffer) {
            Ok(result) => result,
            Err(e) => return Err(AudioError::Stream(format!("loopback read failed: {e}")).into()),
        };
        // Feed the keep-alive stream so the endpoint never suspends.
        let space = keepalive_client
            .get_available_space_in_frames()
            .unwrap_or(0)
            .min(buffer_frame_count);
        if space > 0 {
            let _ = keepalive_render.write_to_device(
                space as usize,
                &silence[..space as usize * block_align],
                None,
            );
        }
        if frames_read == 0 {
            continue;
        }
        let bytes = &buffer[..frames_read as usize * block_align];
        let mut samples = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.as_chunks::<4>().0 {
            samples.push(f32::from_le_bytes(*chunk));
        }
        let audio_chunk = AudioChunk {
            kind: SourceKind::System,
            pts_us: monotonic_us(),
            sample_rate: config.sample_rate,
            channels: config.channels,
            samples,
        };
        let _ = tx.try_send(audio_chunk);
    }

    let _ = audio_client.stop_stream();
    let _ = keepalive_client.stop_stream();
    Ok(())
}

impl AudioSource for WasapiLoopbackSource {
    fn start(&mut self) -> Result<(), AudioError> {
        Ok(()) // the stream starts in `new`; see the trait docs.
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

impl Drop for WasapiLoopbackSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WASAPI loopback only receives packets while the render endpoint
    /// is active. Run this while a sound plays to see chunks flow:
    /// `cargo test -p osr-audio loopback -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a live audio endpoint; play a sound while it runs"]
    fn loopback_delivers_chunks() {
        let mut source = WasapiLoopbackSource::new().expect("loopback source");
        source.start().expect("start");

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let (mut chunks, mut samples) = (0u32, 0usize);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(chunk)) = source.next_chunk(Duration::from_millis(100)) {
                chunks += 1;
                samples += chunk.samples.len();
            }
        }
        println!("loopback delivered {chunks} chunks, {samples} samples in 3s");
        assert!(samples > 0, "loopback delivered no audio — endpoint idle?");
    }
}
