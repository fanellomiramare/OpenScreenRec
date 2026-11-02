//! Windows capture backend: Windows Graphics Capture, via the
//! [`windows-capture`](https://github.com/NiiightmareXD/windows-capture)
//! crate (MIT). WGC requires Windows 10 1903+; frames arrive BGRA on
//! the GPU and are downloaded to CPU memory here.
//!
//! Frames are delivered on the crate's own callback thread through a
//! bounded channel: when the consumer stalls, overflow frames are
//! dropped (`try_send`), which is the documented back-pressure
//! strategy for change-driven capture.

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

/// Runs on the windows-capture callback thread.
struct WgcCallback {
    tx: SyncSender<VideoFrame>,
}

impl GraphicsCaptureApiHandler for WgcCallback {
    type Flags = SyncSender<VideoFrame>;
    type Error = CallbackError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { tx: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let buffer = frame.buffer()?;
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

/// WGC capture of one monitor. TrySendError is deliberately ignored at
/// the call site: dropping frames beats stalling the compositor callback.
pub struct WindowsWgcSource {
    rx: Receiver<VideoFrame>,
    control: Option<CaptureControl<WgcCallback, CallbackError>>,
    size: (u32, u32),
}

impl WindowsWgcSource {
    pub fn new(monitor_id: usize, fps: u32) -> Result<Self, CaptureError> {
        let monitor = if monitor_id == 0 {
            Monitor::primary()
        } else {
            Monitor::from_index(monitor_id)
        }
        .map_err(|e| CaptureError::Start(format!("invalid monitor {monitor_id}: {e}")))?;
        let width = monitor
            .width()
            .map_err(|e| CaptureError::Start(format!("monitor size unavailable: {e}")))?;
        let height = monitor
            .height()
            .map_err(|e| CaptureError::Start(format!("monitor size unavailable: {e}")))?;

        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        // OS-side throttle: frames become eligible at most this often.
        // The real cadence is held by the pipeline (duplicating the last
        // frame when the screen doesn't change).
        let interval = Duration::from_millis((1000 / u64::from(fps.max(1))).max(1));
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(interval),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            tx,
        );
        let control = WgcCallback::start_free_threaded(settings)
            .map_err(|e| CaptureError::Start(format!("failed to start the WGC session: {e}")))?;
        Ok(Self {
            rx,
            control: Some(control),
            size: (width, height),
        })
    }
}

impl FrameSource for WindowsWgcSource {
    fn start(&mut self) -> Result<(), CaptureError> {
        Ok(()) // the WGC session starts in `new`; see the trait docs.
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
