use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

/// Per-OS default global shortcut. `CmdOrCtrl` maps to Cmd on macOS
/// and Ctrl on Windows/Linux, so the same string works everywhere.
pub fn default_shortcut() -> String {
    "CmdOrCtrl+Shift+Y".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Global shortcut that toggles the launcher, e.g. "CmdOrCtrl+Shift+Y".
    pub shortcut: String,
    /// API key for the OpenAI-compatible endpoint (OpenAI, OpenRouter,
    /// Ollama — any value works for local servers).
    #[serde(default)]
    pub openai_api_key: String,
    /// Base URL of the OpenAI-compatible chat endpoint.
    #[serde(default = "default_chat_base_url")]
    pub chat_base_url: String,
    /// Chat model id, e.g. "gpt-oss-120b" (Ollama/OpenRouter/OpenAI).
    #[serde(default = "default_chat_model")]
    pub chat_model: String,
}

fn default_chat_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_chat_model() -> String {
    "gpt-5-mini".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut(),
            openai_api_key: String::new(),
            chat_base_url: default_chat_base_url(),
            chat_model: default_chat_model(),
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    Some(dir.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> Settings {
    match settings_path(app).and_then(|p| fs::read_to_string(p).ok()) {
        Some(raw) => {
            let mut s: Settings = serde_json::from_str(&raw).unwrap_or_default();
            // Backfill fields added in later versions.
            if s.chat_base_url.is_empty()
                || s.chat_base_url == "https://openrouter.ai/api/v1" // old default
            {
                s.chat_base_url = default_chat_base_url();
            }
            if s.chat_model.is_empty() || s.chat_model == "gpt-oss-120b" // old default
            {
                s.chat_model = default_chat_model();
            }
            s
        }
        None => Settings::default(),
    }
}

pub fn save(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("could not resolve app data dir")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

/// Helper for data-file paths (todos, clipboard history, ...).
pub fn data_file(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    Some(dir.join(name))
}
