//! macOS capture backend: ScreenCaptureKit, via the
//! [`screencapturekit`](https://github.com/svtslf/screencapturekit)
//! crate. SCK requires macOS 12.3+ and the Screen Recording permission
//! (TCC); see [`super::screen_recording_granted`].
//!
//! Region capture: SCK has no per-rectangle filter on macOS 12–14, so
//! like the Windows backend we capture the display and crop each frame
//! — here in software, off the locked pixel buffer.
//!
//! Frames are delivered on SCK's own dispatch queue through a bounded
//! channel: overflow frames are dropped (`try_send`), the same
//! back-pressure policy as the other backends.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use screencapturekit::cm::SCFrameStatus;
use screencapturekit::prelude::*;
use screencapturekit::stream::configuration::PixelFormat;

use crate::{CaptureError, FrameFormat, FrameSource};
use osr_common::frame::VideoFrame;
use osr_common::monotonic_us;

/// Frames buffered between the SCK callback queue and the consumer.
const CHANNEL_CAPACITY: usize = 8;

/// Rectangle to crop out of the display frame, physical pixels.
#[derive(Debug, Clone, Copy)]
struct Crop {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct SckHandler {
    tx: Mutex<SyncSender<VideoFrame>>,
    crop: Option<Crop>,
}

impl SCStreamOutputTrait for SckHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if of_type != SCStreamOutputType::Screen {
            return;
        }
        if sample.frame_status() != Some(SCFrameStatus::Complete) {
            return; // idle/blank/suspended frames carry no content
        }
        let Some(pixel_buffer) = sample.pixel_buffer() else {
            return;
        };
        let display_width = pixel_buffer.width() as u32;
        let display_height = pixel_buffer.height() as u32;
        let stride = pixel_buffer.bytes_per_row();
        let Ok(guard) = pixel_buffer.lock_read_only() else {
            return;
        };
        let base = guard.base_address();

        // Copy the buffer (stripping row padding) straight into the
        // frame, cropped if needed — all while the buffer is locked.
        let (width, height, data) = match self.crop {
            None => (
                display_width,
                display_height,
                copy_packed(base, stride, display_width, display_height),
            ),
            Some(crop) => (
                crop.width,
                crop.height,
                copy_cropped(
                    base,
                    stride,
                    display_width,
                    display_height,
                    crop.x,
                    crop.y,
                    crop.width,
                    crop.height,
                ),
            ),
        };
        drop(guard);

        if data.is_empty() {
            return;
        }
        let frame = VideoFrame {
            pts_us: monotonic_us(),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Bgra8,
            data,
        };
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.try_send(frame);
        }
    }
}

fn copy_packed(base: *const u8, stride: usize, width: u32, height: u32) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    if stride == row_bytes {
        let len = row_bytes * height as usize;
        return unsafe { std::slice::from_raw_parts(base, len) }.to_vec();
    }
    copy_cropped(base, stride, width, height, 0, 0, width, height)
}

fn copy_cropped(
    base: *const u8,
    stride: usize,
    display_width: u32,
    display_height: u32,
    crop_x: u32,
    crop_y: u32,
    crop_width: u32,
    crop_height: u32,
) -> Vec<u8> {
    let Some(last_x) = crop_x.checked_add(crop_width) else {
        return Vec::new();
    };
    let Some(last_y) = crop_y.checked_add(crop_height) else {
        return Vec::new();
    };
    if last_x > display_width || last_y > display_height {
        return Vec::new();
    }
    let row_bytes = crop_width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * crop_height as usize);
    for row in crop_y..last_y {
        let start = row as usize * stride + crop_x as usize * 4;
        let row = unsafe { std::slice::from_raw_parts(base.add(start), row_bytes) };
        out.extend_from_slice(row);
    }
    out
}

/// SCK capture of one display (or a cropped region of it).
pub struct SckSource {
    rx: Receiver<VideoFrame>,
    stream: SCStream,
    size: (u32, u32),
}

impl SckSource {
    fn build(crop: Option<Crop>, monitor_index: usize, fps: u32) -> Result<Self, CaptureError> {
        let content = SCShareableContent::get().map_err(|e| {
            CaptureError::NotAuthorized(format!(
                "screen content unavailable — grant Screen Recording permission ({e})"
            ))
        })?;
        let displays = content.displays();
        let display = displays.get(monitor_index).ok_or_else(|| {
            CaptureError::Start(format!(
                "monitor {monitor_index} not found ({} available)",
                displays.len()
            ))
        })?;
        // Physical pixels (Retina-safe): the filter's point dimensions
        // are scaled by the display's pixel scale, queried through
        // CoreGraphics — available on every macOS version, unlike the
        // crate's macos_14-gated `point_pixel_scale`.
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build()
            .map_err(|e| CaptureError::Start(format!("content filter failed: {e}")))?;
        let display_width = physical_pixels_wide(display.display_id());
        let display_height = physical_pixels_high(display.display_id());

        let (width, height, crop) = match crop {
            None => (display_width, display_height, None),
            Some(mut crop) => {
                // Clamp into the display; keep at least 2x2 pixels.
                crop.x = crop.x.min(display_width.saturating_sub(2));
                crop.y = crop.y.min(display_height.saturating_sub(2));
                crop.width = crop.width.clamp(2, display_width - crop.x);
                crop.height = crop.height.clamp(2, display_height - crop.y);
                (crop.width, crop.height, Some(crop))
            }
        };

        let config = SCStreamConfiguration::new()
            .with_width(display_width)
            .with_height(display_height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(true)
            .with_minimum_frame_interval(&CMTime::new(1, fps.max(1) as i32));

        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let mut stream = SCStream::new(&filter, &config)
            .map_err(|e| CaptureError::Start(format!("failed to create the SCK stream: {e}")))?;
        stream
            .add_output_handler(
                SckHandler {
                    tx: Mutex::new(tx),
                    crop,
                },
                SCStreamOutputType::Screen,
            )
            .map_err(|e| CaptureError::Start(format!("failed to attach the handler: {e}")))?;
        stream
            .start_capture()
            .map_err(|e| CaptureError::NotAuthorized(format!("failed to start capture: {e}")))?;

        Ok(Self {
            rx,
            stream,
            size: (width, height),
        })
    }

    pub fn new(monitor_index: usize, fps: u32) -> Result<Self, CaptureError> {
        Self::build(None, monitor_index, fps)
    }

    /// Capture `region` (primary-display-local physical pixels).
    pub fn new_region(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self, CaptureError> {
        let crop = Crop {
            x: x.max(0) as u32,
            y: y.max(0) as u32,
            width,
            height,
        };
        Self::build(Some(crop), 0, fps)
    }
}

impl FrameSource for SckSource {
    fn start(&mut self) -> Result<(), CaptureError> {
        Ok(()) // the SCK stream starts in the constructors; see the trait docs.
    }

    fn stop(&mut self) {
        let _ = self.stream.stop_capture();
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<VideoFrame>, CaptureError> {
        match self.rx.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::Stream(
                "the capture stream closed unexpectedly".into(),
            )),
        }
    }

    fn size(&self) -> Option<(u32, u32)> {
        Some(self.size)
    }
}

impl Drop for SckSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Screen-recording permission (TCC) and display geometry, via
/// CoreGraphics. Preflight returns without prompting; request triggers
/// the system prompt (and opens the Privacy pane on recent versions).
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGDisplayPixelsWide(display_id: u32) -> usize;
    fn CGDisplayPixelsHigh(display_id: u32) -> usize;
}

/// Physical pixel width of a display (1x on standard, 2x on Retina).
fn physical_pixels_wide(display_id: u32) -> u32 {
    // SAFETY: plain getters over a valid display id.
    unsafe { CGDisplayPixelsWide(display_id).max(1) as u32 }
}

/// Physical pixel height of a display.
fn physical_pixels_high(display_id: u32) -> u32 {
    // SAFETY: plain getters over a valid display id.
    unsafe { CGDisplayPixelsHigh(display_id).max(1) as u32 }
}

/// Is Screen Recording already granted? Never prompts.
pub fn screen_recording_granted() -> bool {
    // SAFETY: plain CoreGraphics query, no side effects.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Ask macOS to grant Screen Recording (shows the system prompt /
/// Privacy pane). Returns the granted state right after the call;
/// the user may still be mid-approval, so poll [`screen_recording_granted`].
pub fn request_screen_recording() -> bool {
    // SAFETY: triggers the standard TCC prompt.
    unsafe { CGRequestScreenCaptureAccess() }
}
