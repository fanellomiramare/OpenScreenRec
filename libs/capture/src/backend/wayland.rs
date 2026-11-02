//! Linux Wayland capture backend: the desktop portal ScreenCast
//! interface (xdg-desktop-portal) feeding a PipeWire stream, via
//! `ashpd` and `pipewire`.
//!
//! A dedicated thread owns everything: the portal round-trip (on a
//! short-lived tokio runtime), the PipeWire main loop and the frame
//! pump. The constructor reports the negotiated size back through a
//! channel once the compositor fixates the format.
//!
//! The user picks the source in the compositor's own dialog during
//! construction, so the first start blocks until they do — documented
//! UX trade-off for the MVP. Regions are cropped off the stream
//! frames in output coordinates: exact on single-monitor setups (the
//! picker's assumption), approximate across multi-monitor.
//!
//! Only `BGRx` is negotiated, matching the BGRA frames the rest of the
//! pipeline expects (the fourth byte is left undefined by the format,
//! exactly like X11, and made opaque here). Reference: the official
//! `screen_cast_pw.rs` example in the ashpd repository.

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ashpd::desktop::{
    PersistMode,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
};
use pipewire as pw;
use pw::spa::param::format_utils;
use pw::spa::param::video::VideoFormat;
use pw::spa::pod::Pod;
use pw::spa::utils::SpaTypes;
use pw::{context::ContextBox, main_loop::MainLoopBox, properties::properties, stream::StreamBox};

use crate::{CaptureError, FrameFormat, FrameSource};
use osr_common::frame::VideoFrame;
use osr_common::monotonic_us;

/// Frames buffered between the PipeWire thread and the consumer.
const CHANNEL_CAPACITY: usize = 8;
/// How long the constructor waits: the compositor dialog is on screen.
const INIT_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the compositor gets to fixate a video format.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
struct Crop {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// State the PipeWire callbacks run against (they take `&mut`).
struct Pump {
    tx: SyncSender<VideoFrame>,
    crop: Option<Crop>,
    /// Negotiated geometry, published once via `init_tx` and kept for
    /// the process callback.
    size: Arc<std::sync::Mutex<Option<(u32, u32)>>>,
    /// The negotiated format fourcc, mirrored for the process callback.
    format: Arc<std::sync::Mutex<Option<i32>>>,
    init_tx: std::sync::mpsc::Sender<Result<(u32, u32), String>>,
    warned_format: AtomicU8,
}

impl Pump {
    fn report_size(&self, size: (u32, u32)) {
        let mut slot = self.size.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Some(size);
            let _ = self.init_tx.send(Ok(size));
        }
    }

    fn warn_once(&self, message: &str) {
        if self.warned_format.swap(1, Ordering::Relaxed) == 0 {
            tracing::warn!("{message}");
        }
    }
}

fn pump_frames(pump: &mut Pump, frame_bytes: &[u8], stride: usize, width: u32, height: u32) {
    let (out_width, out_height, data) = match pump.crop {
        None => (
            width,
            height,
            copy_packed(frame_bytes, stride, width, height),
        ),
        Some(crop) => (
            crop.width,
            crop.height,
            copy_cropped(frame_bytes, stride, width, height, crop),
        ),
    };
    if data.is_empty() {
        return;
    }
    let frame = VideoFrame {
        pts_us: monotonic_us(),
        width: out_width,
        height: out_height,
        stride: out_width * 4,
        format: FrameFormat::Bgra8,
        data,
    };
    let _ = pump.tx.try_send(frame);
}

fn copy_rows(bytes: &[u8], stride: usize, width: u32, first_row: u32, rows: u32) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * rows as usize);
    for row in first_row..first_row.saturating_add(rows) {
        let start = row as usize * stride;
        if start + row_bytes > bytes.len() {
            break;
        }
        out.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    out
}

fn copy_packed(bytes: &[u8], stride: usize, width: u32, height: u32) -> Vec<u8> {
    copy_rows(bytes, stride, width, 0, height)
}

fn copy_cropped(bytes: &[u8], stride: usize, width: u32, height: u32, crop: Crop) -> Vec<u8> {
    if crop.y.saturating_add(crop.height) > height || crop.x.saturating_add(crop.width) > width {
        return Vec::new();
    }
    let row_bytes = crop.width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * crop.height as usize);
    for row in crop.y..crop.y + crop.height {
        let start = row as usize * stride + crop.x as usize * 4;
        if start + row_bytes > bytes.len() {
            break;
        }
        out.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    out
}

fn make_opaque(data: &mut [u8]) {
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
}

/// The portal round-trip: session → source selection (the compositor
/// dialog) → PipeWire node id + connection fd.
async fn open_portal() -> Result<(u32, OwnedFd), String> {
    let proxy = Screencast::new().await.map_err(|e| e.to_string())?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(ashpd::enumflags2::BitFlags::from_flag(SourceType::Monitor))
                .set_multiple(false)
                .set_restore_token(None)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|e| e.to_string())?;
    let response = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| format!("portal start failed ({e}) — did you pick a source?"))?
        .response()
        .map_err(|e| e.to_string())?;
    let stream = response
        .streams()
        .first()
        .cloned()
        .ok_or_else(|| "no source was selected".to_string())?;
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| e.to_string())?;
    Ok((stream.pipe_wire_node_id(), fd))
}

/// Everything after the portal: PipeWire stream setup, format
/// negotiation and the frame pump. Runs on the capture thread; blocks
/// in the main loop until the stop flag flips.
fn run_stream(
    stop: &Arc<AtomicBool>,
    tx: SyncSender<VideoFrame>,
    init_tx: std::sync::mpsc::Sender<Result<(u32, u32), String>>,
    crop: Option<Crop>,
) -> Result<(u32, u32), String> {
    pw::init();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime failed: {e}"))?;
    let (node_id, fd) = runtime.block_on(open_portal())?;

    let mainloop = MainLoopBox::new(None).map_err(|e| e.to_string())?;
    let context = ContextBox::new(mainloop.loop_(), None).map_err(|e| e.to_string())?;
    let core = context.connect_fd(fd, None).map_err(|e| e.to_string())?;

    let stream = StreamBox::new(
        &core,
        "openscreenrec",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;

    let size = Arc::new(std::sync::Mutex::new(None));
    let format: Arc<std::sync::Mutex<Option<i32>>> = Arc::new(std::sync::Mutex::new(None));
    let mut pump = Pump {
        tx,
        crop,
        size: Arc::clone(&size),
        format: Arc::clone(&format),
        init_tx: init_tx.clone(),
        warned_format: AtomicU8::new(0),
    };

    // Negotiate exactly one format: BGRx, native size, compositor-paced.
    let params_object = pw::spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx
        ),
    );
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(params_object),
    )
    .map_err(|e| e.to_string())?
    .0
    .into_inner();
    let pod =
        Pod::from_bytes(&values).ok_or_else(|| "failed to serialize the format pod".to_string())?;
    let mut params = [pod];

    let listener = stream
        .add_local_listener_with_user_data(&mut pump)
        .param_changed(|_stream, pump, id, param| {
            let Some(param) = param else { return };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != pw::spa::param::format::MediaType::Video
                || media_subtype != pw::spa::param::format::MediaSubtype::Raw
            {
                return;
            }
            let mut info = pw::spa::param::video::VideoInfoRaw::default();
            let _ = info.parse(param); // SpaSuccess carries nothing we need
            let negotiated = info.format();
            *pump.format.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(negotiated.as_raw() as i32);
            if negotiated == VideoFormat::BGRx {
                pump.report_size((
                    info.size().width.max(0) as u32,
                    info.size().height.max(0) as u32,
                ));
            } else {
                pump.warn_once(&format!(
                    "compositor negotiated {negotiated:?} instead of BGRx; frames will be dropped"
                ));
            }
        })
        .process(|stream, pump| {
            let format = *pump.format.lock().unwrap_or_else(|p| p.into_inner());
            if format != Some(VideoFormat::BGRx.as_raw() as i32) {
                return; // already warned once in param_changed
            }
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return; // out of buffers this tick
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let Some(size) = *pump.size.lock().unwrap_or_else(|p| p.into_inner()) else {
                return;
            };
            let chunk = data.chunk();
            let (offset, size_bytes) = (chunk.offset() as usize, chunk.size() as usize);
            let stride = if chunk.stride() > 0 {
                chunk.stride() as usize
            } else {
                size.0 as usize * 4
            };
            let Some(bytes) = data.data() else { return };
            let end = (offset + size_bytes).min(bytes.len());
            if offset >= end {
                return;
            }
            let mut frame_bytes = bytes[offset..end].to_vec();
            make_opaque(&mut frame_bytes);
            pump_frames(pump, &frame_bytes, stride, size.0, size.1);
        })
        .register()
        .map_err(|e| e.to_string())?;

    stream
        .connect(
            pw::spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| format!("failed to connect the PipeWire stream: {e}"))?;

    let deadline = Instant::now() + FORMAT_TIMEOUT;
    while size.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
        if stop.load(Ordering::Relaxed) || Instant::now() > deadline {
            return Err("the compositor did not negotiate a video format in time".into());
        }
        mainloop
            .loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(50)));
    }
    let size = size
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .expect("size checked above");

    // Drive the loop on this thread until stop() quits it.
    mainloop.run();

    let _ = listener; // keep the listener alive until the loop is done
    drop(stream);
    drop(core);
    drop(context);
    let _ = init_tx;
    Ok(size)
}

/// Capture source over the portal/PipeWire stream.
pub struct WaylandSource {
    rx: Receiver<VideoFrame>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    size: (u32, u32),
}

impl WaylandSource {
    /// Open the portal and capture what the user picks in the dialog.
    /// The constructor blocks until the user chooses (or times out).
    pub fn new(_monitor_index: usize, fps: u32) -> Result<Self, CaptureError> {
        Self::open(None, fps)
    }

    /// Capture a rectangle relative to the chosen output.
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
        Self::open(Some(crop), fps)
    }

    fn open(crop: Option<Crop>, _fps: u32) -> Result<Self, CaptureError> {
        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let (init_tx, init_rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("openscreenrec-wayland".into())
            .spawn(move || {
                if let Err(message) = run_stream(&thread_stop, tx, init_tx.clone(), crop) {
                    let _ = init_tx.send(Err(message));
                }
            })
            .map_err(|e| CaptureError::Start(format!("failed to spawn the capture thread: {e}")))?;

        let size = init_rx
            .recv_timeout(INIT_TIMEOUT)
            .map_err(|_| {
                CaptureError::Start(
                    "the Wayland portal did not answer in time — was the dialog completed?".into(),
                )
            })?
            .map_err(CaptureError::Start)?;

        Ok(Self {
            rx,
            thread: Some(thread),
            stop,
            size,
        })
    }
}

impl FrameSource for WaylandSource {
    fn start(&mut self) -> Result<(), CaptureError> {
        Ok(()) // the stream starts in the constructor; see the trait docs.
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
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

impl Drop for WaylandSource {
    fn drop(&mut self) {
        self.stop();
    }
}
