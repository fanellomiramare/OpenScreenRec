//! Recording session orchestration: capture → encode in one background
//! thread, with a cheap handle the UI can poll, pause or stop.
//!
//! Audio (M2): selected sources are captured alongside the video and
//! written to temporary WAV files; when the recording stops, the mux
//! step combines them into the final MP4 (video stream-copied, audio
//! as AAC). Pause freezes both timelines at once, so A/V stay aligned
//! and paused time is absent from the output — during a pause the
//! sources are drained and discarded.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use osr_audio::{
    AudioSource, WavAudioWriter, create_microphone_source, create_system_audio_source,
};
use osr_capture::FrameSource;
use osr_common::frame::VideoFrame;
use osr_encode::ffmpeg::{FfmpegPipeEncoder, VideoEncoder, mux_video_with_wav};
use osr_encode::{VideoCodec, VideoSettings};

/// Which audio sources to record alongside the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSelection {
    None,
    Microphone,
    System,
    Both,
}

impl AudioSelection {
    fn wants_microphone(self) -> bool {
        matches!(self, AudioSelection::Microphone | AudioSelection::Both)
    }

    fn wants_system(self) -> bool {
        matches!(self, AudioSelection::System | AudioSelection::Both)
    }
}

/// What the user asked to record.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    /// 0 = primary monitor.
    pub monitor_id: usize,
    pub fps: u32,
    pub codec: VideoCodec,
    pub output: PathBuf,
    pub audio: AudioSelection,
}

/// A live recording. Drop it after `request_stop` + draining
/// [`Session::try_result`], or call [`Session::join`] to block.
pub struct Session {
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    recorded_us: Arc<AtomicU64>,
    rx: Receiver<Result<PathBuf, String>>,
    thread: Option<JoinHandle<()>>,
    output: PathBuf,
}

impl Session {
    /// Output file path decided at start time.
    pub fn output(&self) -> &Path {
        &self.output
    }

    /// Recording time excluding pauses — what the output file will span.
    pub fn recorded_elapsed(&self) -> Duration {
        Duration::from_micros(self.recorded_us.load(Ordering::Relaxed))
    }

    /// Ask the recorder thread to finish; it writes the file and reports.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Freeze both timelines (paused time leaves the output entirely).
    pub fn pause(&self) {
        self.pause.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    /// Non-blocking check for the final result.
    pub fn try_result(&mut self) -> Option<Result<PathBuf, String>> {
        self.rx.try_recv().ok()
    }

    /// Stop and wait for the final result.
    pub fn join(mut self) -> Result<PathBuf, String> {
        self.request_stop();
        self.pause.store(false, Ordering::Relaxed);
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
/// duration matches recorded wall-clock time.
pub fn start(request: RecordRequest) -> Result<Session, String> {
    let mut source = osr_capture::backend::create_monitor_source(request.monitor_id, request.fps)
        .map_err(|e| format!("capture unavailable: {e}"))?;
    let (width, height) = source
        .size()
        .ok_or("the capture backend could not report its size")?;
    source
        .start()
        .map_err(|e| format!("failed to start capture: {e}"))?;

    let mut microphone = if request.audio.wants_microphone() {
        let mut mic = create_microphone_source().map_err(|e| {
            format!("microphone unavailable: {e} — record without it or pick another device")
        })?;
        mic.start()
            .map_err(|e| format!("failed to start the microphone: {e}"))?;
        Some(mic)
    } else {
        None
    };
    let mut system = if request.audio.wants_system() {
        match create_system_audio_source() {
            Ok(mut sys) => {
                sys.start()
                    .map_err(|e| format!("failed to start system audio: {e}"))?;
                Some(sys)
            }
            Err(e) => {
                drop(microphone.take()); // the session is cancelled: release the device
                return Err(format!(
                    "system audio unavailable: {e} — record without it or pick another source"
                ));
            }
        }
    } else {
        None
    };

    let with_audio = microphone.is_some() || system.is_some();
    // With audio, the video encoder writes to a temp file that the mux
    // step then combines with the WAVs into the requested output.
    let video_file = if with_audio {
        request.output.with_extension("tmp.mp4")
    } else {
        request.output.clone()
    };

    let settings = VideoSettings {
        width,
        height,
        fps: request.fps,
        codec: request.codec,
        bitrate_bps: None,
    };
    let encoder = FfmpegPipeEncoder::new(settings, &video_file)
        .map_err(|e| format!("failed to start the encoder: {e}"))?;

    let mic_wav = microphone
        .as_ref()
        .map(|mic| WavAudioWriter::create(&mic_wav_path(&request.output), mic.config()))
        .transpose()
        .map_err(|e| format!("failed to create the microphone WAV: {e}"))?;
    let system_wav = system
        .as_ref()
        .map(|sys| WavAudioWriter::create(&system_wav_path(&request.output), sys.config()))
        .transpose()
        .map_err(|e| format!("failed to create the system audio WAV: {e}"))?;

    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let recorded_us = Arc::new(AtomicU64::new(0));
    let output = request.output.clone();
    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(request.fps.max(1)));

    let thread_stop = Arc::clone(&stop);
    let thread_pause = Arc::clone(&pause);
    let thread_recorded = Arc::clone(&recorded_us);
    let thread_output = output.clone();
    let thread = std::thread::Builder::new()
        .name("openscreenrec-recorder".into())
        .spawn(move || {
            let mut state = Loop {
                stop: &thread_stop,
                pause: &thread_pause,
                recorded_us: &thread_recorded,
                interval: frame_interval,
                last_frame: None,
                last_tick: std::time::Instant::now(),
            };
            let result = record_loop(
                &mut *source,
                &mut microphone,
                &mut system,
                encoder,
                &video_file,
                mic_wav,
                system_wav,
                &mut state,
                &thread_output,
            );
            let _ = tx.send(result);
        })
        .map_err(|e| format!("failed to spawn the recorder thread: {e}"))?;

    Ok(Session {
        stop,
        pause,
        recorded_us,
        rx,
        thread: Some(thread),
        output,
    })
}

fn mic_wav_path(output: &Path) -> PathBuf {
    output.with_extension("mic.wav")
}

fn system_wav_path(output: &Path) -> PathBuf {
    output.with_extension("system.wav")
}

/// Per-loop bookkeeping, kept separate to keep `record_loop` readable.
struct Loop<'a> {
    stop: &'a AtomicBool,
    pause: &'a AtomicBool,
    recorded_us: &'a AtomicU64,
    interval: Duration,
    last_frame: Option<VideoFrame>,
    last_tick: std::time::Instant,
}

impl Loop<'_> {
    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn tick(&mut self) {
        if !self.pause.load(Ordering::Relaxed) {
            let elapsed = self.last_tick.elapsed();
            self.recorded_us
                .fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
        }
        self.last_tick = std::time::Instant::now();
    }
}

#[allow(clippy::too_many_arguments)]
fn record_loop(
    source: &mut dyn FrameSource,
    microphone: &mut Option<Box<dyn AudioSource>>,
    system: &mut Option<Box<dyn AudioSource>>,
    mut encoder: FfmpegPipeEncoder,
    video_file: &Path,
    mut mic_wav: Option<WavAudioWriter>,
    mut system_wav: Option<WavAudioWriter>,
    state: &mut Loop<'_>,
    output: &Path,
) -> Result<PathBuf, String> {
    let mut frames_written = 0u64;
    let outcome = loop {
        if state.stop_requested() {
            break Ok(());
        }

        if state.pause.load(Ordering::Relaxed) {
            // Freeze both timelines: drain and discard whatever arrives,
            // so resume continues from fresh data and in sync.
            let _ = source.next_frame(Duration::from_millis(20));
            if let Some(mic) = microphone.as_mut() {
                while matches!(mic.next_chunk(Duration::ZERO), Ok(Some(_))) {}
            }
            if let Some(sys) = system.as_mut() {
                while matches!(sys.next_chunk(Duration::ZERO), Ok(Some(_))) {}
            }
            state.last_tick = std::time::Instant::now();
            continue;
        }

        match source.next_frame(state.interval) {
            Ok(Some(frame)) => {
                if let Err(e) = encoder.write_frame(&frame) {
                    break Err(format!("encoding failed: {e}"));
                }
                frames_written += 1;
                state.last_frame = Some(frame);
            }
            Ok(None) => {
                // No repaint this tick: hold the cadence by repeating
                // the last frame (keeps duration accurate on static screens).
                if let Some(frame) = &state.last_frame {
                    if let Err(e) = encoder.write_frame(frame) {
                        break Err(format!("encoding failed: {e}"));
                    }
                    frames_written += 1;
                }
            }
            Err(e) => break Err(format!("capture failed: {e}")),
        }

        if let (Some(mic), Some(wav)) = (microphone.as_mut(), mic_wav.as_mut())
            && let Err(e) = drain_chunks(mic.as_mut(), wav)
        {
            break Err(e);
        }
        if let (Some(sys), Some(wav)) = (system.as_mut(), system_wav.as_mut())
            && let Err(e) = drain_chunks(sys.as_mut(), wav)
        {
            break Err(e);
        }

        state.tick();
    };

    source.stop();
    if let Some(mic) = microphone.as_mut() {
        mic.stop();
    }
    if let Some(sys) = system.as_mut() {
        sys.stop();
    }

    let video_done = match outcome {
        Ok(()) => match encoder.finish() {
            Ok(path) => path,
            Err(e) => return Err(format!("encoding failed: {e}")),
        },
        Err(message) => {
            drop(encoder);
            cleanup_temps(video_file, mic_wav, system_wav);
            return Err(message);
        }
    };

    let has_mic = mic_wav.is_some();
    let has_system = system_wav.is_some();
    finalize_wavs(mic_wav, system_wav);

    if !has_mic && !has_system {
        tracing::info!(
            "recording finished: {} ({frames_written} frames)",
            video_done.display()
        );
        return Ok(video_done);
    }

    // Collect the recorded WAVs, dropping tracks that captured (almost)
    // nothing: muxing an empty audio track with -shortest would truncate
    // the whole video.
    let mut audio_paths = Vec::new();
    if has_mic {
        audio_paths.push(mic_wav_path(output));
    }
    if has_system {
        audio_paths.push(system_wav_path(output));
    }
    let usable: Vec<PathBuf> = audio_paths
        .into_iter()
        .filter(|path| {
            let non_empty = std::fs::metadata(path)
                .map(|m| m.len() > 512)
                .unwrap_or(false);
            if !non_empty {
                tracing::warn!(
                    "audio track {} captured no samples; skipping it",
                    path.display()
                );
                let _ = std::fs::remove_file(path);
            }
            non_empty
        })
        .collect();

    if usable.is_empty() {
        // Nothing usable was captured: promote the video and finish.
        std::fs::rename(&video_done, output).map_err(|e| format!("failed to finalize: {e}"))?;
        tracing::warn!("no audio captured; saved video only");
        tracing::info!(
            "recording finished: {} ({frames_written} frames)",
            output.display()
        );
        return Ok(output.to_path_buf());
    }

    // Mix the audio into the video. `video_file` is the temp MP4; the
    // mux writes the requested output and the temps go away.
    let audio_refs: Vec<&Path> = usable.iter().map(Path::new).collect();
    match mux_video_with_wav(video_file, &audio_refs, output) {
        Ok(path) => {
            cleanup_files([video_file]);
            cleanup_files(&usable);
            tracing::info!(
                "recording finished: {} ({frames_written} frames, {} audio track(s))",
                path.display(),
                usable.len()
            );
            Ok(path)
        }
        Err(e) => {
            cleanup_files([video_file]);
            cleanup_files(&usable);
            Err(format!("mux failed: {e}"))
        }
    }
}

/// Read every chunk already buffered by a source into its WAV writer.
fn drain_chunks(source: &mut dyn AudioSource, writer: &mut WavAudioWriter) -> Result<(), String> {
    loop {
        match source.next_chunk(Duration::ZERO) {
            Ok(Some(chunk)) => {
                writer
                    .write(&chunk)
                    .map_err(|e| format!("audio writing failed: {e}"))?;
            }
            Ok(None) => return Ok(()),
            Err(e) => return Err(format!("audio capture failed: {e}")),
        }
    }
}

fn finalize_wavs(mic_wav: Option<WavAudioWriter>, system_wav: Option<WavAudioWriter>) {
    if let Some(wav) = mic_wav
        && let Err(e) = wav.finalize()
    {
        tracing::warn!("microphone WAV finalize failed: {e}");
    }
    if let Some(wav) = system_wav
        && let Err(e) = wav.finalize()
    {
        tracing::warn!("system audio WAV finalize failed: {e}");
    }
}

fn cleanup_files(paths: impl IntoIterator<Item = impl AsRef<Path>>) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

fn cleanup_temps(
    video_file: &Path,
    mic_wav: Option<WavAudioWriter>,
    system_wav: Option<WavAudioWriter>,
) {
    // Finalizing (or dropping) the writers leaves the files on disk;
    // remove everything this session created.
    drop(mic_wav);
    drop(system_wav);
    let _ = std::fs::remove_file(video_file);
}
