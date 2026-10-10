use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::paths;

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
    /// Chat model id, e.g. "gpt-5-mini" (Ollama/OpenRouter/OpenAI).
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

fn settings_path() -> Option<PathBuf> {
    paths::data_file("settings.json")
}

pub fn load() -> Settings {
    match settings_path().and_then(|p| fs::read_to_string(p).ok()) {
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

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = settings_path().ok_or("could not resolve app data dir")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Shortcut helpers
// ---------------------------------------------------------------------------

/// Canonicalize a shortcut string to "Cmd+Shift+Y" style: modifier order
/// fixed (Cmd/Ctrl, Shift, Alt) and duplicates removed.
pub fn normalize_shortcut(raw: &str) -> String {
    let mut parts: Vec<String> = raw
        .split('+')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    let rank = |p: &str| match p.to_lowercase().as_str() {
        "cmdorctrl" | "cmd" | "command" | "ctrl" | "control" => 0,
        "shift" => 1,
        "alt" | "option" | "opt" => 2,
        _ => 3,
    };
    parts.sort_by_key(|p| rank(p));

    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let lower = p.to_lowercase();
        let canonical = match lower.as_str() {
            "cmd" | "command" | "super" | "meta" | "cmdorctrl" => "Cmd",
            "ctrl" | "control" => "Ctrl",
            "shift" => "Shift",
            "alt" | "option" | "opt" => "Alt",
            _ => p.as_str(),
        };
        if !out.contains(&canonical.to_string()) {
            out.push(canonical.to_string());
        }
    }
    out.join("+")
}

/// Pretty-print a shortcut string for display (⌘ ⇧ Y on macOS).
pub fn pretty_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|p| {
            let l = p.to_lowercase();
            match l.as_str() {
                "cmdorctrl" => {
                    if cfg!(target_os = "macos") {
                        "⌘".to_string()
                    } else {
                        "Ctrl".to_string()
                    }
                }
                "cmd" | "command" => "⌘".to_string(),
                "ctrl" | "control" => "Ctrl".to_string(),
                "shift" => "Shift".to_string(),
                "alt" | "option" | "opt" => {
                    if cfg!(target_os = "macos") {
                        "Option".to_string()
                    } else {
                        "Alt".to_string()
                    }
                }
                _ => p.to_uppercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}
