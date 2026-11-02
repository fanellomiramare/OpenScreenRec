//! Shared utilities for the OpenScreenRec crates and application.
//!
//! Keep this crate small: configuration types, well-known paths, the
//! pipeline-wide clock, shared A/V frame types and trivial helpers only.
//! Anything with a hardware or OS dependency belongs in a dedicated
//! crate (`osr-capture`, `osr-audio`, ...).

pub mod frame;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Local;

/// Human-readable application name, used in window titles and logs.
pub const APP_NAME: &str = "OpenScreenRec";

/// Lower-case identifier used for config and data directories.
pub const APP_ID: &str = "openscreenrec";

/// Prefix of generated recording file names.
pub const RECORDING_PREFIX: &str = "OpenScreenRec";

/// Epoch of the pipeline-wide monotonic clock.
static CLOCK_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Microseconds on a process-wide monotonic clock.
///
/// Video frames and audio chunks both stamp their `pts_us` here, so
/// mixing, pause/resume and A/V sync share one timeline.
pub fn monotonic_us() -> u64 {
    let epoch = CLOCK_EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_micros() as u64
}

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

/// File name for a new recording, e.g. `OpenScreenRec_20261102_213000.mp4`.
pub fn timestamped_recording_name(extension: &str) -> String {
    format!(
        "{RECORDING_PREFIX}_{}.{}",
        Local::now().format("%Y%m%d_%H%M%S"),
        extension.trim_start_matches('.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_clock_moves_forward() {
        let first = monotonic_us();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = monotonic_us();
        assert!(second > first);
    }

    #[test]
    fn config_dir_is_namespaced() {
        let dir = config_dir().expect("a config dir exists on all supported platforms");
        assert!(dir.ends_with(APP_ID));
    }

    #[test]
    fn default_recordings_dir_resolves() {
        assert!(default_recordings_dir().is_some());
    }

    #[test]
    fn recording_names_are_timestamped() {
        let name = timestamped_recording_name(".mp4");
        assert!(name.starts_with("OpenScreenRec_"));
        assert!(name.ends_with(".mp4"));
        let stamp = name
            .trim_start_matches("OpenScreenRec_")
            .trim_end_matches(".mp4");
        assert_eq!(stamp.len(), 15, "YYYYMMDD_HHMMSS, got {stamp}");
        assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '_'));
    }
}
