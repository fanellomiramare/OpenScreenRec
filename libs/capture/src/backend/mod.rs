//! Platform capture backends.
//!
//! One module per OS (and per Linux display server), compiled only
//! where it applies. Every backend implements [`crate::FrameSource`];
//! the application picks a backend through this module's factories —
//! never through `#[cfg]` in UI code.
//!
//! References: the backend design follows the crate structure of
//! [`windows-capture`](https://github.com/NiiightmareXD/windows-capture)
//! (Windows), [`screencapturekit`](https://github.com/svtslf/screencapturekit)
//! (macOS), and the official [`ashpd`](https://github.com/bilelmoussaoui/ashpd)
//! screencast example (Wayland). All MIT or Apache-2.0 licensed.

#[cfg(target_os = "macos")]
mod sck;
#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;
#[cfg(target_os = "windows")]
mod wgc;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

#[cfg(target_os = "windows")]
pub const BACKEND_NAME: &str = "windows-graphics-capture";

#[cfg(target_os = "macos")]
pub const BACKEND_NAME: &str = "screen-capture-kit";

#[cfg(all(unix, not(target_os = "macos")))]
pub const BACKEND_NAME: &str = "x11-getimage / wayland-portal";

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

/// Is this session running under Wayland? Decides which Linux backend
/// handles the capture (X11 is the fallback when DISPLAY is set).
#[cfg(all(unix, not(target_os = "macos")))]
fn session_is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Create a [`FrameSource`] for any supported [`CaptureTarget`].
#[cfg(any(target_os = "windows", target_os = "macos", unix))]
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
#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
pub fn create_capture_source(
    _target: &crate::CaptureTarget,
    _fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    Err(crate::CaptureError::Unsupported(
        "capture is not supported on this platform".into(),
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

/// Linux: Wayland goes through the portal (the user picks the source in
/// the compositor dialog), X11 grabs the whole desktop off the root
/// window.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn create_monitor_source(
    monitor_id: usize,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    if session_is_wayland() {
        tracing::info!("Wayland session detected: using the desktop portal");
        Ok(Box::new(wayland::WaylandSource::new(monitor_id, fps)?))
    } else {
        tracing::info!("X11 session detected: using XGetImage");
        Ok(Box::new(x11::X11Source::new(monitor_id, fps)?))
    }
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

/// Linux: same dispatch as monitors; regions crop the frames.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn create_region_source(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Box<dyn crate::FrameSource>, crate::CaptureError> {
    if session_is_wayland() {
        tracing::info!("Wayland session detected: using the desktop portal");
        Ok(Box::new(wayland::WaylandSource::new_region(
            x, y, width, height, fps,
        )?))
    } else {
        tracing::info!("X11 session detected: using XGetImage");
        Ok(Box::new(x11::X11Source::new_region(
            x, y, width, height, fps,
        )?))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_name_is_reported() {
        assert!(!super::backend_name().is_empty());
    }
}
