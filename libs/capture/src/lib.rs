//! Screen and window capture for OpenScreenRec.
//!
//! This crate exposes one platform-independent trait, [`FrameSource`],
//! implemented by a backend per operating system:
//!
//! | OS      | API                                                        | Backend status |
//! | ------- | ---------------------------------------------------------- | -------------- |
//! | Windows | Windows Graphics Capture (via `windows-capture`)           | planned for M1 |
//! | macOS   | ScreenCaptureKit (via `screencapturekit`)                  | planned for M4 |
//! | Linux   | XSHM (X11) / desktop portal + PipeWire (Wayland)           | planned for M5 |
//!
//! Backends land milestone by milestone; until then backend queries
//! report [`CaptureError::Unsupported`].

pub mod backend;
pub mod frame;

use std::time::Duration;

pub use frame::{FrameFormat, VideoFrame};

/// What to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    /// One whole monitor.
    Monitor {
        /// Index into the virtual desktop, in backend enumeration order.
        id: usize,
    },
    /// A rectangle in global (virtual desktop) coordinates, physical pixels.
    Region {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// A single OS window, identified by the backend.
    Window { id: u64 },
}

/// Why starting or running a capture failed.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// No backend implements the requested capture on this OS (yet).
    #[error("capture backend not implemented on this platform: {0}")]
    Unsupported(String),
    /// The OS denied access (e.g. macOS screen-recording permission).
    #[error("screen capture permission not granted: {0}")]
    NotAuthorized(String),
    /// The backend could not start (device busy, target gone, ...).
    #[error("capture failed to start: {0}")]
    Start(String),
    /// The stream stopped unexpectedly.
    #[error("capture stream failed: {0}")]
    Stream(String),
}

/// A source of video frames: one running capture session.
///
/// Backends deliver frames as fast as the OS composites them (typically
/// vsync-locked); rate limiting to the configured FPS happens upstream
/// in the pipeline, not inside the backend.
pub trait FrameSource: Send {
    /// Begin delivering frames. Must be called before [`Self::next_frame`].
    fn start(&mut self) -> Result<(), CaptureError>;

    /// Stop the session and release OS resources.
    fn stop(&mut self);

    /// Block up to `timeout` for the next frame. `Ok(None)` means no
    /// frame arrived in time; the caller decides whether to keep waiting.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<VideoFrame>, CaptureError>;
}

/// Enumerate the capture targets visible to the current backend
/// (monitors and windows). Backends fill this in as they land (M1+).
pub fn list_targets() -> Result<Vec<CaptureTarget>, CaptureError> {
    Err(CaptureError::Unsupported(
        "target enumeration arrives with the first platform backend".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_targets_is_an_honest_stub_until_m1() {
        assert!(matches!(list_targets(), Err(CaptureError::Unsupported(_))));
    }
}
