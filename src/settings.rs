//! Persistent user settings for fast-resume.
//!
//! Settings live in a small JSON file under the config directory
//! (`~/.config/fast-resume/settings.json`). Reads and writes are best-effort: a
//! missing or malformed file falls back to defaults and write failures are
//! ignored, so the TUI never breaks over a settings problem.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::config_dir;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    /// Preview pane share of the side-by-side layout, in percent. `None` until
    /// the user resizes the preview, after which the last size is remembered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_ratio: Option<u16>,
}

impl Settings {
    /// Load settings from the default config path, falling back to defaults.
    pub fn load() -> Self {
        Self::load_from(&settings_file())
    }

    /// Load settings from `path`, returning defaults if it is missing or invalid.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Persist settings to the default config path (best-effort).
    pub fn save(&self) {
        self.save_to(&settings_file());
    }

    /// Persist settings to `path`, creating the parent directory as needed.
    /// Errors are ignored so a settings problem never interrupts the app.
    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_from_missing_file_falls_back_to_defaults() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("does-not-exist.json");

        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn load_from_corrupt_file_falls_back_to_defaults() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, "not json {{{").unwrap();

        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn save_to_then_load_from_round_trips_preview_ratio() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("nested").join("settings.json");
        let settings = Settings {
            preview_ratio: Some(52),
        };

        settings.save_to(&path);

        assert_eq!(Settings::load_from(&path), settings);
    }
}
