//! Platform capture backends.
//!
//! One module per OS, compiled only where it applies. Every backend
//! implements [`crate::FrameSource`]; the application picks a backend
//! through this module's factories — never through `#[cfg]` in UI code.
//!
//! References: the backend design follows the crate structure of
//! [`windows-capture`](https://github.com/NiiightmareXD/windows-capture)
//! (Windows), [`screencapturekit`](https://github.com/svtslf/screencapturekit)
//! (macOS, planned) and [`scap`](https://github.com/CapSoftware/scap)
//! (cross-platform model). All MIT or Apache-2.0 licensed.

#[cfg(target_os = "macos")]
mod sck;
#[cfg(target_os = "windows")]
mod wgc;

#[cfg(target_os = "windows")]
pub const BACKEND_NAME: &str = "windows-graphics-capture";

#[cfg(target_os = "macos")]
pub const BACKEND_NAME: &str = "screen-capture-kit";

#[cfg(all(unix, not(target_os = "macos")))]
pub const BACKEND_NAME: &str = "x11 / pipewire-portal (M5)";

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
pub const BACKEND_NAME: &str = "unsupported";

/// Human-readable name of the backend available on this OS.
pub fn backend_name() -> &'static str {
    BACKEND_NAME
}

/// Is screen recording already permitted on this OS? Always `true`
/// where no permission system exists (Windows, Linux).
pub fn screen_recording_granted() -> bool {
    #[cfg(target_os = "macos")]
    return sck::screen_recording_granted();
    #[cfg(not(target_os = "macos"))]
    true
}

/// Ask the OS for screen recording permission (shows the system prompt
/// on macOS; no-op elsewhere). Returns the granted state right after
/// the call — poll [`screen_recording_granted`] for the final answer.
pub fn request_screen_recording_permission() -> bool {
    #[cfg(target_os = "macos")]
    return sck::request_screen_recording();
    #[cfg(not(target_os = "macos"))]
    true
}

/// Create a [`FrameSource`] for any supported [`CaptureTarget`].
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn create_capture_source(
    target: &crate::CaptureTarget,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    match target {
        crate::CaptureTarget::Monitor { id } => create_monitor_source(*id, fps),
        crate::CaptureTarget::Region {
            x,
            y,
            width,
            height,
        } => create_region_source(*x, *y, *width, *height, fps),
        crate::CaptureTarget::Window { .. } => Err(crate::CaptureError::Unsupported(
            "window capture is planned for M6".into(),
        )),
    }
}

/// Create a [`FrameSource`] for any supported [`CaptureTarget`].
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn create_capture_source(
    _target: &crate::CaptureTarget,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "capture lands on Linux in M5".into(),
    ))
}

/// Create a [`FrameSource`] for one monitor at a given frame-rate hint.
///
/// `monitor_id` 0 is the primary monitor. The returned source may
/// already be delivering frames (backends may start their OS session
/// in the constructor); call [`FrameSource::start`] anyway, then drive
/// [`FrameSource::next_frame`] and finish with [`FrameSource::stop`].
#[cfg(target_os = "windows")]
pub fn create_monitor_source(
    monitor_id: usize,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Ok(Box::new(wgc::WindowsWgcSource::new(monitor_id, fps)?))
}

/// Create a [`FrameSource`] for one monitor at a given frame-rate hint.
#[cfg(target_os = "macos")]
pub fn create_monitor_source(
    monitor_id: usize,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Ok(Box::new(sck::SckSource::new(monitor_id, fps)?))
}

/// Monitor capture lands on Linux in M5.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn create_monitor_source(
    _monitor_id: usize,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "monitor capture lands on Linux in M5".into(),
    ))
}

/// Create a [`FrameSource`] for a rectangle in primary-monitor-local
/// physical pixels, by capturing its monitor and cropping each frame.
#[cfg(target_os = "windows")]
pub fn create_region_source(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Ok(Box::new(wgc::WindowsWgcSource::new_region(
        x, y, width, height, fps,
    )?))
}

/// Create a [`FrameSource`] for a rectangle in primary-display-local
/// physical pixels.
#[cfg(target_os = "macos")]
pub fn create_region_source(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Ok(Box::new(sck::SckSource::new_region(
        x, y, width, height, fps,
    )?))
}

/// Region capture lands on Linux in M5.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn create_region_source(
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "region capture lands on Linux in M5".into(),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_name_is_reported() {
        assert!(!super::backend_name().is_empty());
    }
}
