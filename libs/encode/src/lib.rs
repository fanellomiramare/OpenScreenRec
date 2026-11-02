//! Video and audio encoding for OpenScreenRec.
//!
//! M1–M3 ship with an external FFmpeg process fed over a pipe (kept out
//! of the source tree, so the license stays permissive). The long-term
//! plan (M6+) is native hardware encoders per OS — Media Foundation /
//! NVENC (Windows), VideoToolbox (macOS), VAAPI / NVENC (Linux) — with
//! pure-Rust container writing via `osr-mux`.

/// Compression formats the pipeline can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

impl VideoCodec {
    /// The `ffmpeg -c:v` name for this codec.
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "libx264",
            VideoCodec::Hevc => "libx265",
            VideoCodec::Av1 => "libsvtav1",
        }
    }
}

/// Quality and geometry knobs shared by every encoder backend.
#[derive(Debug, Clone)]
pub struct VideoSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: VideoCodec,
    /// Target bitrate in bits per second; `None` leaves the encoder in
    /// quality mode (CRF-like), which is the default.
    pub bitrate_bps: Option<u32>,
}

/// Errors produced by encoder backends.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The backend doesn't exist on this OS (yet).
    #[error("encoder backend not implemented yet: {0}")]
    Unsupported(String),
    #[error("encoder rejected the input: {0}")]
    Input(String),
    #[error("encoder process failed: {0}")]
    Process(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_codec_names_are_stable() {
        assert_eq!(VideoCodec::H264.ffmpeg_name(), "libx264");
        assert_eq!(VideoCodec::Hevc.ffmpeg_name(), "libx265");
        assert_eq!(VideoCodec::Av1.ffmpeg_name(), "libsvtav1");
    }
}
