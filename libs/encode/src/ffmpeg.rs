//! FFmpeg pipe encoder: raw BGRA frames into an external `ffmpeg`
//! process that compresses and muxes to the target container.
//!
//! The FFmpeg binary stays out of our source tree, which keeps the
//! project MIT-clean; users get it from their package manager, or we
//! bundle it in installers (decided at M5). This mirrors how OBS and
//! GPU Screen Recorder drive external encoders.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::{EncodeError, VideoSettings};
use osr_common::frame::{FrameFormat, VideoFrame};

/// Anything that turns video frames into a finished output file.
pub trait VideoEncoder {
    /// Encode one frame. Frames must arrive at a steady cadence (the
    /// pipeline owns the pacing); the encoder assigns timestamps
    /// sequentially.
    fn write_frame(&mut self, frame: &VideoFrame) -> Result<(), EncodeError>;

    /// Signal end-of-stream, wait for the process, finalize the file.
    fn finish(self) -> Result<PathBuf, EncodeError>;
}

/// H.264/HEVC/AV1 encoding through an external FFmpeg process.
pub struct FfmpegPipeEncoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_sink: Arc<Mutex<Vec<u8>>>,
    output: PathBuf,
}

impl FfmpegPipeEncoder {
    /// Locate the ffmpeg executable: `$OSR_FFMPEG_PATH` if set,
    /// otherwise `ffmpeg` on the PATH.
    pub fn ffmpeg_binary() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("OSR_FFMPEG_PATH") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
        let path_var = std::env::var_os("PATH")?;
        let exe = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        std::env::split_paths(&path_var)
            .map(|dir| dir.join(exe))
            .find(|candidate| candidate.is_file())
    }

    /// Spawn ffmpeg and open the frame pipe.
    ///
    /// Odd dimensions are padded up (yuv420p needs even geometry) so
    /// exotic monitor sizes don't break the encoder.
    pub fn new(settings: VideoSettings, output: impl Into<PathBuf>) -> Result<Self, EncodeError> {
        let output = output.into();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let Some(ffmpeg) = Self::ffmpeg_binary() else {
            return Err(EncodeError::Process(
                "ffmpeg not found — install it or point OSR_FFMPEG_PATH at the binary".into(),
            ));
        };

        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", "bgra"])
            .args(["-s", &format!("{}x{}", settings.width, settings.height)])
            .args(["-r", &settings.fps.to_string()])
            .args(["-i", "pipe:0"])
            .args(["-an", "-c:v", settings.codec.ffmpeg_name()])
            .args(["-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"]);
        if !settings.width.is_multiple_of(2) || !settings.height.is_multiple_of(2) {
            command.args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]);
        }
        command
            .args(["-movflags", "+faststart"])
            .arg(&output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| EncodeError::Process(format!("failed to spawn ffmpeg: {e}")))?;
        let stdin = child.stdin.take();
        let stderr_sink = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let sink = Arc::clone(&stderr_sink);
            std::thread::spawn(move || {
                // Draining on a side thread keeps ffmpeg from ever
                // blocking on a full stderr pipe.
                let _ = stderr.read_to_end(&mut sink.lock().unwrap_or_else(|p| p.into_inner()));
            });
        }

        Ok(Self {
            child,
            stdin,
            stderr_sink,
            output,
        })
    }

    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(
            &self
                .stderr_sink
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )
        .trim()
        .to_string()
    }
}

impl VideoEncoder for FfmpegPipeEncoder {
    fn write_frame(&mut self, frame: &VideoFrame) -> Result<(), EncodeError> {
        if frame.format != FrameFormat::Bgra8 {
            return Err(EncodeError::Input("only BGRA frames are supported".into()));
        }
        let expected_len = frame.width as usize * frame.height as usize * 4;
        if frame.data.len() < expected_len {
            return Err(EncodeError::Input(format!(
                "frame buffer too small: {} bytes for {}x{}x4",
                frame.data.len(),
                frame.width,
                frame.height
            )));
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(EncodeError::Process(
                "the frame pipe is closed (encoder already finalized?)".into(),
            ));
        };
        if frame.stride as usize == expected_len {
            stdin.write_all(&frame.data)?;
        } else {
            // Strip the stride padding row by row.
            let row_bytes = frame.row_bytes();
            let stride = frame.stride as usize;
            let mut packed = Vec::with_capacity(expected_len);
            for row in 0..frame.height as usize {
                let start = row * stride;
                packed.extend_from_slice(&frame.data[start..start + row_bytes]);
            }
            stdin.write_all(&packed)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<PathBuf, EncodeError> {
        let output = self.output.clone();
        drop(self.stdin.take()); // EOF tells ffmpeg to finalize the container
        let status = self
            .child
            .wait()
            .map_err(|e| EncodeError::Process(format!("failed to wait for ffmpeg: {e}")))?;
        if !status.success() {
            return Err(EncodeError::Process(format!(
                "ffmpeg exited with {status}: {}",
                self.stderr_text()
            )));
        }
        Ok(output)
    }
}

impl Drop for FfmpegPipeEncoder {
    fn drop(&mut self) {
        // If the encoder is dropped without `finish`, tear the process
        // down instead of leaking it.
        drop(self.stdin.take());
        if self
            .child
            .try_wait()
            .map(|done| done.is_none())
            .unwrap_or(false)
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VideoCodec;

    /// Skip on runners without ffmpeg (Windows CI has none; local dev
    /// and Linux CI do).
    fn ffmpeg_available() -> bool {
        FfmpegPipeEncoder::ffmpeg_binary().is_some()
            || std::env::var_os("OSR_FFMPEG_PATH").is_some()
    }

    #[test]
    fn ffmpeg_pipe_writes_a_real_mp4() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not available on this runner");
            return;
        }

        let (width, height, fps) = (320u32, 240u32, 10u32);
        let dir = std::env::temp_dir().join(format!("osr-encode-test-{}", std::process::id()));
        let output = dir.join("test.mp4");

        let settings = VideoSettings {
            width,
            height,
            fps,
            codec: VideoCodec::H264,
            bitrate_bps: None,
        };
        let mut encoder = FfmpegPipeEncoder::new(settings, &output).expect("encoder starts");

        // One second of solid, slowly changing color.
        for i in 0..fps {
            let frame = VideoFrame {
                pts_us: u64::from(i) * 100_000,
                width,
                height,
                stride: width * 4,
                format: FrameFormat::Bgra8,
                data: vec![i as u8; (width * height * 4) as usize],
            };
            encoder.write_frame(&frame).expect("frame written");
        }
        let path = encoder.finish().expect("finalized");
        assert_eq!(path, output);

        let len = std::fs::metadata(&path).expect("output file exists").len();
        assert!(len > 1_000, "suspiciously small MP4: {len} bytes");

        std::fs::remove_dir_all(dir).ok();
    }
}
