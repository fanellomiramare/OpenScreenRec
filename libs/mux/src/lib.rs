//! Container writing for OpenScreenRec.
//!
//! The trait keeps the pipeline storage-agnostic: the FFmpeg-based
//! encoder of M1 writes the output file itself, while native encoder
//! backends (M6+) push compressed samples through a [`Muxer`].

/// Output container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
    Webm,
}

impl Container {
    /// File extension including the leading dot.
    pub fn file_extension(self) -> &'static str {
        match self {
            Container::Mp4 => ".mp4",
            Container::Mkv => ".mkv",
            Container::Webm => ".webm",
        }
    }
}

/// Errors from muxer backends.
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    /// The backend doesn't exist (yet).
    #[error("muxer backend not implemented yet: {0}")]
    Unsupported(String),
    #[error("failed to write the output file: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_extensions() {
        assert_eq!(Container::Mp4.file_extension(), ".mp4");
        assert_eq!(Container::Mkv.file_extension(), ".mkv");
        assert_eq!(Container::Webm.file_extension(), ".webm");
    }
}
