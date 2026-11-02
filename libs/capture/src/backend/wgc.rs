//! Windows capture backend: Windows Graphics Capture, via the
//! [`windows-capture`](https://github.com/NiiightmareXD/windows-capture)
//! crate (MIT). WGC requires Windows 10 1903+; frames arrive BGRA on
//! the GPU and are downloaded to CPU memory here.
//!
//! Frames are delivered on the crate's own callback thread through a
//! bounded channel: when the consumer stalls, overflow frames are
//! dropped (`try_send`), which is the documented back-pressure
//! strategy for change-driven capture.
//!
//! Region capture: WGC captures whole monitors, so a region request
//! captures its monitor and crops every frame on the GPU
//! (`Frame::buffer_crop`).

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::{CaptureError, FrameFormat, FrameSource};
use osr_common::frame::VideoFrame;
use osr_common::monotonic_us;

/// Frames buffered between the capture callback thread and the consumer.
const CHANNEL_CAPACITY: usize = 8;

type CallbackError = Box<dyn std::error::Error + Send + Sync>;

/// Rectangle to crop out of the monitor frame, physical pixels.
#[derive(Debug, Clone, Copy)]
struct Crop {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Handed to the callback thread at session start.
struct WgcFlags {
    tx: SyncSender<VideoFrame>,
    crop: Option<Crop>,
}

/// Runs on the windows-capture callback thread, carrying the channel
/// and optional crop rectangle handed over through the settings flags.
struct WgcStateful {
    tx: SyncSender<VideoFrame>,
    crop: Option<Crop>,
}

impl GraphicsCaptureApiHandler for WgcStateful {
    type Flags = WgcFlags;
    type Error = CallbackError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            tx: ctx.flags.tx,
            crop: ctx.flags.crop,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let buffer = match self.crop {
            Some(crop) => {
                frame.buffer_crop(crop.x, crop.y, crop.x + crop.width, crop.y + crop.height)?
            }
            None => frame.buffer()?,
        };
        let width = buffer.width();
        let height = buffer.height();
        // `as_nopadding_buffer` removes the row pitch when the GPU
        // texture is padded; the result is tightly packed BGRA.
        let mut scratch = Vec::new();
        let packed = buffer.as_nopadding_buffer(&mut scratch);
        let video_frame = VideoFrame {
            pts_us: monotonic_us(),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Bgra8,
            data: packed.to_vec(),
        };
        let _ = self.tx.try_send(video_frame);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// WGC capture of one monitor (or a cropped region of it). Frame drops
/// via `try_send` are the deliberate back-pressure strategy: dropping
/// frames beats stalling the compositor callback.
pub struct WindowsWgcSource {
    rx: Receiver<VideoFrame>,
    control: Option<CaptureControl<WgcStateful, CallbackError>>,
    size: (u32, u32),
}

impl WindowsWgcSource {
    fn spawn_session(
        item: Monitor,
        crop: Option<Crop>,
        fps: u32,
        size: (u32, u32),
    ) -> Result<Self, CaptureError> {
        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        // OS-side throttle: frames become eligible at most this often.
        // The real cadence is held by the pipeline (duplicating the last
        // frame when the screen doesn't change).
        let interval = Duration::from_millis((1000 / u64::from(fps.max(1))).max(1));
        let settings = Settings::new(
            item,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(interval),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            WgcFlags { tx, crop },
        );
        let control = WgcStateful::start_free_threaded(settings)
            .map_err(|e| CaptureError::Start(format!("failed to start the WGC session: {e}")))?;
        Ok(Self {
            rx,
            control: Some(control),
            size,
        })
    }

    pub fn new(monitor_id: usize, fps: u32) -> Result<Self, CaptureError> {
        let monitor = monitor_by_id(monitor_id)?;
        let size = monitor_size(&monitor)?;
        Self::spawn_session(monitor, None, fps, size)
    }

    /// Capture `region` (primary-monitor-local physical pixels), by
    /// capturing the primary monitor and cropping each frame.
    pub fn new_region(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self, CaptureError> {
        let monitor = Monitor::primary()
            .map_err(|e| CaptureError::Start(format!("no primary monitor: {e}")))?;
        let (monitor_width, monitor_height) = monitor_size(&monitor)?;
        // Clamp into the monitor; keep at least 2x2 pixels of area.
        let crop_x = x.clamp(0, monitor_width as i32 - 2) as u32;
        let crop_y = y.clamp(0, monitor_height as i32 - 2) as u32;
        let crop_width = width.clamp(2, monitor_width - crop_x);
        let crop_height = height.clamp(2, monitor_height - crop_y);
        let crop = Crop {
            x: crop_x,
            y: crop_y,
            width: crop_width,
            height: crop_height,
        };
        Self::spawn_session(monitor, Some(crop), fps, (crop_width, crop_height))
    }
}

fn monitor_by_id(monitor_id: usize) -> Result<Monitor, CaptureError> {
    if monitor_id == 0 {
        Monitor::primary()
    } else {
        Monitor::from_index(monitor_id)
    }
    .map_err(|e| CaptureError::Start(format!("invalid monitor {monitor_id}: {e}")))
}

fn monitor_size(monitor: &Monitor) -> Result<(u32, u32), CaptureError> {
    let width = monitor
        .width()
        .map_err(|e| CaptureError::Start(format!("monitor size unavailable: {e}")))?;
    let height = monitor
        .height()
        .map_err(|e| CaptureError::Start(format!("monitor size unavailable: {e}")))?;
    Ok((width, height))
}

impl FrameSource for WindowsWgcSource {
    fn start(&mut self) -> Result<(), CaptureError> {
        Ok(()) // the WGC session starts in the constructors; see the trait docs.
    }

    fn stop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
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

impl Drop for WindowsWgcSource {
    fn drop(&mut self) {
        self.stop();
    }
}
