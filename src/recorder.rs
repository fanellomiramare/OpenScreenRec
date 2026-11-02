//! Recording session orchestration: capture → encode in one background
//! thread, with a cheap handle the UI can poll or stop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use osr_capture::FrameSource;
use osr_common::frame::VideoFrame;
use osr_encode::ffmpeg::{FfmpegPipeEncoder, VideoEncoder};
use osr_encode::{VideoCodec, VideoSettings};

/// What the user asked to record.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    /// 0 = primary monitor.
    pub monitor_id: usize,
    pub fps: u32,
    pub codec: VideoCodec,
    pub output: PathBuf,
}

/// A live recording. Drop it after `request_stop` + draining
/// [`Session::try_result`], or call [`Session::join`] to block.
pub struct Session {
    stop: Arc<AtomicBool>,
    rx: Receiver<Result<PathBuf, String>>,
    thread: Option<JoinHandle<()>>,
    output: PathBuf,
    started_at: std::time::Instant,
}

impl Session {
    /// Output file path decided at start time.
    pub fn output(&self) -> &Path {
        &self.output
    }

    /// Wall-clock time since the session started.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Ask the recorder thread to finish; it writes the file and reports.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Non-blocking check for the final result.
    pub fn try_result(&mut self) -> Option<Result<PathBuf, String>> {
        self.rx.try_recv().ok()
    }

    /// Stop and wait for the final result.
    pub fn join(mut self) -> Result<PathBuf, String> {
        self.request_stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.rx
            .recv()
            .unwrap_or_else(|_| Err("the recorder thread ended unexpectedly".into()))
    }
}

/// Start recording on a dedicated thread.
///
/// The screen doesn't repaint at a fixed rate, so the loop holds the
/// requested cadence by duplicating the last frame when the OS doesn't
/// deliver a new one in time — the output keeps a constant FPS and the
/// duration matches wall-clock time.
pub fn start(request: RecordRequest) -> Result<Session, String> {
    let mut source = osr_capture::backend::create_monitor_source(request.monitor_id, request.fps)
        .map_err(|e| format!("capture unavailable: {e}"))?;
    let (width, height) = source
        .size()
        .ok_or("the capture backend could not report its size")?;
    source
        .start()
        .map_err(|e| format!("failed to start capture: {e}"))?;

    let settings = VideoSettings {
        width,
        height,
        fps: request.fps,
        codec: request.codec,
        bitrate_bps: None,
    };
    let encoder = FfmpegPipeEncoder::new(settings, &request.output)
        .map_err(|e| format!("failed to start the encoder: {e}"))?;

    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let output = request.output.clone();
    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(request.fps.max(1)));

    let thread = std::thread::Builder::new()
        .name("openscreenrec-recorder".into())
        .spawn(move || {
            let result = record_loop(&mut *source, encoder, frame_interval, &thread_stop);
            let _ = tx.send(result);
        })
        .map_err(|e| format!("failed to spawn the recorder thread: {e}"))?;

    Ok(Session {
        stop,
        rx,
        thread: Some(thread),
        output,
        started_at: std::time::Instant::now(),
    })
}

fn record_loop(
    source: &mut dyn FrameSource,
    mut encoder: FfmpegPipeEncoder,
    frame_interval: Duration,
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    let mut frames_written = 0u64;
    let mut last_frame: Option<VideoFrame> = None;

    let outcome = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match source.next_frame(frame_interval) {
            Ok(Some(frame)) => {
                if let Err(e) = encoder.write_frame(&frame) {
                    break Err(format!("encoding failed: {e}"));
                }
                frames_written += 1;
                last_frame = Some(frame);
            }
            Ok(None) => {
                // No repaint this tick: hold the cadence by repeating
                // the last frame (this is what keeps duration accurate
                // on static screens).
                if let Some(frame) = &last_frame {
                    if let Err(e) = encoder.write_frame(frame) {
                        break Err(format!("encoding failed: {e}"));
                    }
                    frames_written += 1;
                }
            }
            Err(e) => break Err(format!("capture failed: {e}")),
        }
    };

    source.stop();

    match outcome {
        Ok(()) => match encoder.finish() {
            Ok(path) => {
                tracing::info!(
                    "recording finished: {} ({frames_written} frames)",
                    path.display()
                );
                Ok(path)
            }
            Err(e) => Err(format!("encoding failed: {e}")),
        },
        Err(message) => Err(message),
    }
}
