//! Settings + status files — the IPC contract with the (future) omarchy
//! settings plugin.
//!
//! - Panel writes  ~/.config/heyclicky/settings.json  → we hot-reload it
//! - We write      ~/.local/state/heyclicky/status.json → panel displays it
//!
//! Both are plain JSON so the QML side needs nothing but FileView.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// capture + transcribe voice while the hotkey is held
    pub voice_enabled: bool,
    /// model file name inside a models dir (see find_model)
    pub model: String,
    /// whisper language code ("en", "auto", ...)
    pub language: String,
    /// buddy float distance right/down of the pointer (logical px)
    pub buddy_gap: i32,
    /// buddy triangle fill color as hex, e.g. "#A78BFA"
    #[serde(default = "default_cursor_color")]
    pub cursor_color: String,
}

fn default_cursor_color() -> String {
    "#A78BFA".into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            voice_enabled: true,
            model: "ggml-base.en.bin".into(),
            language: "en".into(),
            buddy_gap: 8,
            cursor_color: default_cursor_color(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/heyclicky/settings.json")
}

pub fn status_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/state/heyclicky/status.json")
}

pub fn load() -> Settings {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(txt) => serde_json::from_str(&txt).unwrap_or_else(|e| {
            eprintln!("SETTINGS: bad json in {:?} ({}), using defaults", path, e);
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

pub fn save(s: &Settings) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(&path, json + "\n");
    }
}

/// mtime of the settings file (0 if missing) — polled to hot-reload.
pub fn config_mtime() -> u64 {
    std::fs::metadata(config_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// --- model discovery -------------------------------------------------------
// heyclicky's own dir wins; voxtype's models are reused when present.

pub fn model_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    vec![
        PathBuf::from(&home).join(".local/share/heyclicky/models"),
        PathBuf::from(&home).join(".local/share/voxtype/models"),
    ]
}

/// Resolve a model file name to an existing path across known model dirs.
pub fn find_model(name: &str) -> Option<PathBuf> {
    model_dirs()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// All model file names available (deduped, sorted) — for the settings UI.
pub fn list_models() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for dir in model_dirs() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".bin") && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names
}

// --- status publishing ------------------------------------------------------

#[derive(Serialize)]
pub struct Status {
    pub listening: bool,
    pub transcribing: bool,
    /// true while an agent job runs (agent leg); panel pulses its green dot
    /// off this — QML already reads it (`s.agent_busy === true`).
    pub agent_busy: bool,
    pub model: String,
    pub last_transcript: String,
}

pub fn publish_status(s: &Status) {
    let path = status_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(s) {
        let _ = std::fs::write(&path, json + "\n");
    }
}
