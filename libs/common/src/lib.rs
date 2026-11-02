//! Shared utilities for the OpenScreenRec crates and application.
//!
//! Keep this crate small: configuration types, well-known paths and
//! trivial helpers only. Anything with a hardware or OS dependency
//! belongs in a dedicated crate (`osr-capture`, `osr-audio`, ...).

use std::path::PathBuf;

/// Human-readable application name, used in window titles and logs.
pub const APP_NAME: &str = "OpenScreenRec";

/// Lower-case identifier used for config and data directories.
pub const APP_ID: &str = "openscreenrec";

/// Directory holding the user configuration
/// (`%APPDATA%\openscreenrec` on Windows, `~/.config/openscreenrec` on
/// Linux, `~/Library/Application Support/openscreenrec` on macOS).
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_ID))
}

/// Default directory for recordings: the user's Videos folder,
/// falling back to the home directory.
pub fn default_recordings_dir() -> Option<PathBuf> {
    dirs::video_dir().or_else(dirs::home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_is_namespaced() {
        let dir = config_dir().expect("a config dir exists on all supported platforms");
        assert!(dir.ends_with(APP_ID));
    }

    #[test]
    fn default_recordings_dir_resolves() {
        assert!(default_recordings_dir().is_some());
    }
}
