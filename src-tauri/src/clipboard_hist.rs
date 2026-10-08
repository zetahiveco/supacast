use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const MAX_HISTORY: usize = 50;
const POLL_INTERVAL_MS: u64 = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipEntry {
    pub text: String,
    pub at: DateTime<Local>,
}

type History = Mutex<Vec<ClipEntry>>;

fn history_path(app: &AppHandle) -> Option<PathBuf> {
    super::settings::data_file(app, "clipboard.json")
}

pub fn load(app: &AppHandle) -> Vec<ClipEntry> {
    match history_path(app).and_then(|p| fs::read_to_string(p).ok()) {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => Vec::new(),
    }
}

fn persist(app: &AppHandle, entries: &[ClipEntry]) {
    if let Some(path) = history_path(app) {
        if let Ok(raw) = serde_json::to_string(entries) {
            let _ = fs::write(path, raw);
        }
    }
}

/// Background poller: keeps a rolling history of copied text.
pub fn spawn_clipboard_loop(app: AppHandle) {
    app.manage(History::new(load(&app)));

    std::thread::spawn(move || {
        let mut last_text: Option<String> = None;

        loop {
            let current = read_clipboard(&app);
            if let Some(text) = current {
                if last_text.as_deref() != Some(text.as_str()) {
                    let entries = {
                        let hist = app.state::<History>();
                        let mut hist = hist.lock().unwrap();
                        if hist.iter().any(|e| e.text == text) {
                            None
                        } else {
                            hist.insert(0, ClipEntry { text: text.clone(), at: Local::now() });
                            hist.truncate(MAX_HISTORY);
                            Some(hist.clone())
                        }
                    };
                    if let Some(entries) = entries {
                        persist(&app, &entries);
                    }
                    last_text = Some(text);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
        }
    });
}

fn read_clipboard(app: &AppHandle) -> Option<String> {
    app.clipboard()
        .read_text()
        .ok()
        .filter(|t| !t.trim().is_empty())
}

pub fn get_history(app: &AppHandle) -> Vec<ClipEntry> {
    app.state::<History>().lock().unwrap().clone()
}

pub fn copy_to_clipboard(app: &AppHandle, text: &str) -> Result<(), String> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| e.to_string())
}
