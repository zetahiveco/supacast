use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::paths;

const MAX_HISTORY: usize = 50;
const POLL_INTERVAL_MS: u64 = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipEntry {
    pub text: String,
    pub at: DateTime<Local>,
}

/// In-memory rolling history, shared between the poller thread and the UI.
static HISTORY: Mutex<Option<Vec<ClipEntry>>> = Mutex::new(None);

fn history_path() -> Option<PathBuf> {
    paths::data_file("clipboard.json")
}

fn with_history<R>(f: impl FnOnce(&mut Vec<ClipEntry>) -> R) -> Option<R> {
    let mut guard = HISTORY.lock().unwrap();
    let hist = guard.get_or_insert_with(|| {
        match history_path().and_then(|p| fs::read_to_string(p).ok()) {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            None => Vec::new(),
        }
    });
    Some(f(hist))
}

fn persist(entries: &[ClipEntry]) {
    if let Some(path) = history_path() {
        if let Ok(raw) = serde_json::to_string(entries) {
            let _ = fs::write(path, raw);
        }
    }
}

/// Background poller: keeps a rolling history of copied text. Replaces the
/// Tauri clipboard-manager plugin with a direct `arboard` poller.
pub fn spawn_clipboard_loop() {
    std::thread::spawn(|| {
        // One clipboard handle for the lifetime of the thread (important on
        // Windows where creating handles frequently is expensive).
        let mut clipboard = arboard::Clipboard::new().ok();

        loop {
            if let Some(clip) = clipboard.as_mut() {
                if let Ok(text) = clip.get_text() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        let changed = with_history(|hist| {
                            if hist.first().map(|e| e.text == text).unwrap_or(false) {
                                return false; // fast path: same as newest entry
                            }
                            if hist.iter().any(|e| e.text == text) {
                                return false; // already known, don't reshuffle
                            }
                            hist.insert(0, ClipEntry { text: text.clone(), at: Local::now() });
                            hist.truncate(MAX_HISTORY);
                            true
                        })
                        .unwrap_or(false);
                        if changed {
                            let snapshot =
                                with_history(|hist| hist.clone()).unwrap_or_default();
                            persist(&snapshot);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
        }
    });
}

pub fn get_history() -> Vec<ClipEntry> {
    with_history(|hist| hist.clone()).unwrap_or_default()
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text.to_string()).map_err(|e| e.to_string())
}
