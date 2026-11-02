//! Persisted user settings, stored as TOML in the user config dir.
//!
//! Missing or corrupt files fall back to defaults; saving is
//! best-effort (a failed write must never lose a recording).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Capture and output frame rate.
    pub fps: u32,
    /// Record the microphone alongside the video.
    pub microphone: bool,
    /// Record system audio (loopback) alongside the video.
    pub system_audio: bool,
    /// Seconds of countdown before recording starts; 0 disables it.
    pub countdown_secs: u32,
    /// Where recordings are saved; `None` uses the user's Videos folder.
    pub output_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fps: 30,
            microphone: false,
            system_audio: true,
            countdown_secs: 0,
            output_dir: None,
        }
    }
}

impl Settings {
    /// Path of the settings file, when a config dir is available.
    pub fn file_path() -> Option<PathBuf> {
        super::config_dir().map(|dir| dir.join("settings.toml"))
    }

    /// Load settings, falling back to defaults when absent or unreadable.
    pub fn load() -> Self {
        let text = Self::file_path().and_then(|path| std::fs::read_to_string(path).ok());
        match text {
            Some(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("settings file unreadable, using defaults: {e}");
                Self::default()
            }),
            None => Self::default(),
        }
    }

    /// Persist the settings; failures are logged, never fatal.
    pub fn save(&self) {
        let Some(path) = Self::file_path() else {
            return;
        };
        let serialized = toml::to_string_pretty(self).unwrap_or_default();
        let result =
            std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
                .and_then(|()| std::fs::write(&path, serialized));
        if let Err(e) = result {
            tracing::warn!("could not save settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_toml() {
        let settings = Settings {
            fps: 60,
            microphone: true,
            system_audio: false,
            countdown_secs: 3,
            output_dir: Some(PathBuf::from("D:/Recordings")),
        };
        let text = toml::to_string(&settings).unwrap();
        let parsed: Settings = toml::from_str(&text).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn unknown_fields_and_missing_entries_fall_back_to_defaults() {
        let parsed: Settings = toml::from_str("fps = 45\nunknown_field = 1").unwrap();
        assert_eq!(parsed.fps, 45);
        assert!(parsed.system_audio); // default
    }
}
