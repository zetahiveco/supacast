use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

/// Per-OS default global shortcut. `CmdOrCtrl` maps to Cmd on macOS
/// and Ctrl on Windows/Linux, so the same string works everywhere.
pub fn default_shortcut() -> String {
    "CmdOrCtrl+Shift+Y".to_string()
}

/// API providers offered in Settings. OpenAI is the default; OpenRouter
/// is the alternative; Ollama is the local option. (The frontend keeps
/// its own copy of this list — this one documents the canonical values.)
#[allow(dead_code)]
pub const PROVIDERS: [&str; 3] = ["openai", "openrouter", "ollama"];

/// Default base URL for a provider. Selecting a provider in Settings
/// switches the endpoint to its default.
pub fn provider_base_url(provider: &str) -> &'static str {
    match provider.to_lowercase().as_str() {
        "openrouter" => "https://openrouter.ai/api/v1",
        // Ollama's OpenAI-compatible endpoint on the default local port.
        "ollama" => "http://localhost:11434/v1",
        _ => "https://api.openai.com/v1",
    }
}

fn default_provider() -> String {
    "openai".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Global shortcut that toggles the launcher, e.g. "CmdOrCtrl+Shift+Y".
    pub shortcut: String,
    /// API key for the OpenAI-compatible endpoint (OpenAI, OpenRouter,
    /// Ollama — any value works for local servers).
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default = "default_provider")]
    pub chat_provider: String,
    /// Base URL of the OpenAI-compatible chat endpoint (derived from the
    /// selected provider; provider_base_url is the source of truth).
    #[serde(default = "default_chat_base_url")]
    pub chat_base_url: String,
    /// Chat model id, e.g. "gpt-oss-120b" (Ollama/OpenRouter/OpenAI).
    #[serde(default = "default_chat_model")]
    pub chat_model: String,
    /// Provider for audio transcription ("openai" | "openrouter").
    #[serde(default = "default_provider")]
    pub transcribe_provider: String,
    /// Base URL for transcription (derived from transcribe_provider).
    #[serde(default = "default_chat_base_url")]
    pub transcribe_base_url: String,
    /// Transcription model id, e.g. "gpt-4o-mini-transcribe".
    #[serde(default = "default_transcribe_model")]
    pub transcribe_model: String,
    /// How often an overdue, incomplete todo re-notifies until done, in
    /// minutes. `None`/`Some(0)` = notify once, never repeat. Default 30.
    #[serde(default = "default_remind_repeat")]
    pub remind_repeat_min: Option<u64>,
}

fn default_remind_repeat() -> Option<u64> {
    Some(30)
}

/// The repeat-frequency choices offered in Settings: (label, minutes).
/// minutes == 0 means "notify once, never repeat". (The frontend keeps its
/// own copy of this list — this one documents the canonical values.)
#[allow(dead_code)]
pub const REPEAT_OPTIONS: [(&str, u64); 7] = [
    ("10 min", 10),
    ("15 min", 15),
    ("30 min", 30),
    ("45 min", 45),
    ("1 hour", 60),
    ("2 hours", 120),
    ("Never", 0),
];

/// Label for a `remind_repeat_min` value ("10 min" … "2 hours", "Never").
#[allow(dead_code)]
pub fn repeat_label(mins: Option<u64>) -> &'static str {
    match mins {
        None | Some(0) => "Never",
        Some(m) => REPEAT_OPTIONS
            .iter()
            .find(|(_, v)| *v == m)
            .map(|(l, _)| *l)
            .unwrap_or("30 min"),
    }
}

fn default_chat_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_chat_model() -> String {
    "gpt-5-mini".to_string()
}

fn default_transcribe_model() -> String {
    "gpt-4o-mini-transcribe".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut(),
            openai_api_key: String::new(),
            chat_provider: default_provider(),
            chat_base_url: default_chat_base_url(),
            chat_model: default_chat_model(),
            transcribe_provider: default_provider(),
            transcribe_base_url: default_chat_base_url(),
            transcribe_model: default_transcribe_model(),
            remind_repeat_min: default_remind_repeat(),
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
            if s.chat_provider.trim().is_empty() {
                s.chat_provider = default_provider();
            }
            if s.transcribe_provider.trim().is_empty() {
                s.transcribe_provider = default_provider();
            }
            if s.chat_base_url.is_empty()
                || s.chat_base_url == "https://openrouter.ai/api/v1" // old default
            {
                s.chat_base_url = provider_base_url(&s.chat_provider).to_string();
            }
            if s.chat_model.is_empty() || s.chat_model == "gpt-oss-120b" // old default
            {
                s.chat_model = default_chat_model();
            }
            if s.transcribe_base_url.is_empty() {
                s.transcribe_base_url = provider_base_url(&s.transcribe_provider).to_string();
            }
            if s.transcribe_model.is_empty() {
                s.transcribe_model = default_transcribe_model();
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
