//! Small, human-readable settings stored next to the app's platform config.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub last_folder: Option<PathBuf>,
    pub volume: f32,
    pub show_filenames: bool,
    pub ffmpeg_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_folder: None,
            volume: 0.85,
            show_filenames: false,
            ffmpeg_path: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        let mut settings: Self = serde_json::from_slice(&bytes).unwrap_or_default();
        settings.volume = settings.volume.clamp(0.0, 1.0);
        settings
    }

    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(json) = serde_json::to_vec_pretty(self) else {
            return;
        };
        let _ = fs::write(path, json);
    }
}

fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/com.panolix.photoroulette")
                .join("settings.json"),
        )
    }

    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
        Some(
            PathBuf::from(base)
                .join("PhotoRoulette")
                .join("settings.json"),
        )
    }

    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("photo-roulette").join("settings.json"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            last_folder: Some("/photos/cringe".into()),
            volume: 0.42,
            show_filenames: true,
            ffmpeg_path: Some("/opt/homebrew/bin/ffmpeg".into()),
        };
        let json = serde_json::to_string(&settings).expect("serialize settings");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize settings");

        assert_eq!(restored.last_folder, settings.last_folder);
        assert!((restored.volume - settings.volume).abs() < f32::EPSILON);
        assert!(restored.show_filenames);
        assert_eq!(restored.ffmpeg_path, settings.ffmpeg_path);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let restored: Settings = serde_json::from_str("{\"volume\": 2.0}").expect("parse");
        assert_eq!(restored.volume, 2.0);
        assert!(!restored.show_filenames);
        assert_eq!(restored.last_folder, None);
    }
}
