//! Linux X11 capture backend: XGetImage over `x11rb` — pure Rust, no
//! C dependencies and nothing to install beyond a running X server.
//!
//! Pull-based: [`FrameSource::next_frame`] grabs a frame on demand, so
//! the frame cadence lives entirely in the pipeline. Region requests
//! read the exact sub-rectangle, which makes the server do the crop.
//!
//! Performance note: this uses plain XGetImage (one full-frame copy per
//! call). The MIT-SHM extension would avoid the copy; that optimization
//! is deliberately deferred until the Linux backend is functionally
//! verified in the wild.

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Screen};
use x11rb::rust_connection::RustConnection;

use crate::{CaptureError, FrameFormat, FrameSource};
use osr_common::frame::VideoFrame;
use osr_common::monotonic_us;

pub struct X11Source {
    conn: RustConnection,
    root: u32,
    /// The sub-rectangle of the root window to grab, pixels.
    rect: (i32, i32, u32, u32),
    size: (u32, u32),
}

impl X11Source {
    /// Capture the whole virtual desktop. `monitor_index` selects the
    /// screen; X11 exposes the desktop through one root window, so the
    /// per-monitor split arrives with multi-monitor work (M6).
    pub fn new(monitor_index: usize, _fps: u32) -> Result<Self, CaptureError> {
        let (conn, screen_index) = connect()?;
        let (root, size) = {
            let screen = screen_at(&conn, screen_index)?;
            (
                screen.root,
                (
                    u32::from(screen.width_in_pixels),
                    u32::from(screen.height_in_pixels),
                ),
            )
        };
        if monitor_index != 0 {
            tracing::warn!(
                "monitor {monitor_index} requested: X11 captures the whole desktop for now"
            );
        }
        Ok(Self {
            conn,
            root,
            rect: (0, 0, size.0, size.1),
            size,
        })
    }

    /// Capture a rectangle in root-window coordinates (pixels) — the
    /// server crops for us.
    pub fn new_region(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        _fps: u32,
    ) -> Result<Self, CaptureError> {
        let (conn, screen_index) = connect()?;
        let (root, screen_w, screen_h) = {
            let screen = screen_at(&conn, screen_index)?;
            (
                screen.root,
                u32::from(screen.width_in_pixels),
                u32::from(screen.height_in_pixels),
            )
        };
        // Clamp into the screen, keeping at least 2x2 pixels.
        let x = x.clamp(0, screen_w as i32 - 2);
        let y = y.clamp(0, screen_h as i32 - 2);
        let width = width.clamp(2, screen_w.saturating_sub(x as u32));
        let height = height.clamp(2, screen_h.saturating_sub(y as u32));
        Ok(Self {
            conn,
            root,
            rect: (x, y, width, height),
            size: (width, height),
        })
    }

    fn grab(&mut self) -> Result<VideoFrame, CaptureError> {
        let (x, y, width, height) = self.rect;
        let (x, y) = (i16::try_from(x).unwrap_or(0), i16::try_from(y).unwrap_or(0));
        let (width, height) = (
            u16::try_from(width).unwrap_or(u16::MAX),
            u16::try_from(height).unwrap_or(u16::MAX),
        );
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, x, y, width, height, !0)
            .map_err(|e| CaptureError::Stream(format!("XGetImage failed: {e}")))?
            .reply()
            .map_err(|e| CaptureError::Stream(format!("XGetImage failed: {e}")))?;
        let mut data = reply.data;
        // X11 leaves the fourth byte of each BGRX pixel undefined:
        // turn the buffer into opaque BGRA.
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let width = u32::from(width);
        let height = u32::from(height);
        Ok(VideoFrame {
            pts_us: monotonic_us(),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Bgra8,
            data,
        })
    }
}

fn connect() -> Result<(RustConnection, usize), CaptureError> {
    x11rb::connect(None).map_err(|e| {
        CaptureError::Start(format!(
            "X11 connection failed ({e}) — is DISPLAY set and an X server running?"
        ))
    })
}

fn screen_at(conn: &RustConnection, index: usize) -> Result<&Screen, CaptureError> {
    conn.setup()
        .roots
        .get(index)
        .ok_or_else(|| CaptureError::Start("X11 screen not found".into()))
}

impl FrameSource for X11Source {
    fn start(&mut self) -> Result<(), CaptureError> {
        Ok(())
    }

    fn stop(&mut self) {}

    fn next_frame(&mut self, _timeout: Duration) -> Result<Option<VideoFrame>, CaptureError> {
        Ok(Some(self.grab()?))
    }

    fn size(&self) -> Option<(u32, u32)> {
        Some(self.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Functional X11 capture against whatever X server is available —
    /// CI runs it under Xvfb with a gray root window (see
    /// .github/workflows/ci.yml). Skips when there is no DISPLAY.
    #[test]
    fn x11_grabs_a_frame() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipping: no X server available");
            return;
        }
        let mut source = X11Source::new(0, 30).expect("X11 source opens");
        source.start().expect("start");
        let frame = source
            .next_frame(Duration::from_secs(2))
            .expect("grab succeeds")
            .expect("a frame arrives");
        assert!(frame.width > 0 && frame.height > 0);
        assert!(frame.data.iter().any(|b| *b != 0), "frame is all-black");
    }
}
