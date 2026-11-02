//! Platform capture backends.
//!
//! One module per OS, compiled only where it applies. Every backend
//! implements [`crate::FrameSource`]; the application picks a backend
//! through this module — never through `#[cfg]` in UI code.

#[cfg(target_os = "windows")]
pub const BACKEND_NAME: &str = "windows-graphics-capture (M1)";

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

#[cfg(test)]
mod tests {
    #[test]
    fn backend_name_is_reported() {
        assert!(!super::backend_name().is_empty());
    }
}
