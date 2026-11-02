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

#[cfg(target_os = "windows")]
mod wgc;

#[cfg(target_os = "windows")]
pub const BACKEND_NAME: &str = "windows-graphics-capture";

#[cfg(target_os = "macos")]
pub const BACKEND_NAME: &str = "screen-capture-kit (M4)";

#[cfg(all(unix, not(target_os = "macos")))]
pub const BACKEND_NAME: &str = "x11 / pipewire-portal (M5)";

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
pub const BACKEND_NAME: &str = "unsupported";

/// Human-readable name of the backend available on this OS.
pub fn backend_name() -> &'static str {
    BACKEND_NAME
}

/// Create a [`FrameSource`] for any supported [`CaptureTarget`].
#[cfg(target_os = "windows")]
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
#[cfg(not(target_os = "windows"))]
pub fn create_capture_source(
    _target: &crate::CaptureTarget,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "capture lands on macOS in M4 and on Linux in M5".into(),
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
///
/// Monitor capture lands on macOS in M4 and on Linux in M5.
#[cfg(not(target_os = "windows"))]
pub fn create_monitor_source(
    _monitor_id: usize,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "monitor capture lands on macOS in M4 and on Linux in M5".into(),
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

/// Create a [`FrameSource`] for a rectangle; lands with the platform
/// backends (macOS M4, Linux M5).
#[cfg(not(target_os = "windows"))]
pub fn create_region_source(
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "region capture lands on macOS in M4 and on Linux in M5".into(),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_name_is_reported() {
        assert!(!super::backend_name().is_empty());
    }
}
