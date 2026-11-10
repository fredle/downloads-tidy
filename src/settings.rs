//! Small persisted preferences in %LOCALAPPDATA%\DownloadsTidy\settings.json.
//! Add app-specific fields here, always with `#[serde(default)]` so older
//! settings files keep loading.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// Check for updates in the background and install them silently.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Unix seconds of the last background update check, to throttle to once
    /// a day.
    #[serde(default)]
    pub last_update_check: u64,
    /// Folder to tidy. Empty means the default Downloads folder.
    #[serde(default)]
    pub folder: String,
    #[serde(default = "crate::rules::default_rules")]
    pub rules: Vec<crate::rules::Rule>,
    /// Run automatically while the app is open (including minimised).
    #[serde(default)]
    pub schedule_enabled: bool,
    #[serde(default = "default_interval")]
    pub schedule_minutes: u32,
    /// Leave files modified within this many minutes alone.
    #[serde(default = "default_quiet")]
    pub quiet_minutes: u32,
}

fn default_true() -> bool {
    true
}

fn default_interval() -> u32 {
    60
}

fn default_quiet() -> u32 {
    10
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            last_update_check: 0,
            folder: String::new(),
            rules: crate::rules::default_rules(),
            schedule_enabled: false,
            schedule_minutes: default_interval(),
            quiet_minutes: default_quiet(),
        }
    }
}

/// The usual Downloads location; users with a relocated folder pick their own.
pub fn default_folder() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home)
        .join("Downloads")
        .to_string_lossy()
        .into_owned()
}

pub fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into()))
        .join("DownloadsTidy")
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(settings_path(), json);
    }
}
