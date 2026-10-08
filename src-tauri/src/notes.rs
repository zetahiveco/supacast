use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub created: Option<chrono::DateTime<Local>>,
}

fn notes_path(app: &AppHandle) -> Option<PathBuf> {
    super::settings::data_file(app, "notes.json")
}

pub fn load_all(app: &AppHandle) -> Vec<Note> {
    match notes_path(app).and_then(|p| fs::read_to_string(p).ok()) {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => Vec::new(),
    }
}

pub fn save_all(app: &AppHandle, notes: &[Note]) -> Result<(), String> {
    let path = notes_path(app).ok_or("could not resolve app data dir")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(notes).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

pub fn add(app: &AppHandle, text: &str) -> Result<Note, String> {
    let mut notes = load_all(app);
    let note = Note {
        id: uuid::Uuid::new_v4().to_string(),
        text: text.trim().to_string(),
        created: Some(Local::now()),
    };
    notes.push(note.clone());
    save_all(app, &notes)?;
    Ok(note)
}

pub fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut notes = load_all(app);
    notes.retain(|n| n.id != id);
    save_all(app, &notes)
}

/// List notes, newest first. Optional `query` filters by substring.
pub fn list(app: &AppHandle, query: Option<&str>) -> Vec<Note> {
    let q = query.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let mut notes: Vec<Note> = load_all(app)
        .into_iter()
        .filter(|n| {
            q.is_empty() || n.text.to_lowercase().contains(&q)
        })
        .collect();

    // Newest first.
    notes.sort_by_key(|n| std::cmp::Reverse(n.created));
    notes
}
