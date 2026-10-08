use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;

/// One saved dictation: the transcript plus its recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dictation {
    pub id: String,
    pub text: String,
    /// Agent reply for Dictate (Supacast) recordings; none for plain text.
    #[serde(default)]
    pub answer: Option<String>,
    /// MIME type the recorder produced (audio/webm, audio/mp4, …).
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub created: Option<chrono::DateTime<Local>>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

fn meta_path(app: &AppHandle) -> Option<PathBuf> {
    super::settings::data_file(app, "dictations.json")
}

fn audio_dir(app: &AppHandle) -> Option<PathBuf> {
    super::settings::data_file(app, "dictations")
}

/// File extension for the recorder's MIME type.
fn ext_for_mime(mime: &str) -> &str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/mp4" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        _ => "webm",
    }
}

fn load_all(app: &AppHandle) -> Vec<Dictation> {
    match meta_path(app).and_then(|p| fs::read_to_string(p).ok()) {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => Vec::new(),
    }
}

fn save_all(app: &AppHandle, dictations: &[Dictation]) -> Result<(), String> {
    let path = meta_path(app).ok_or("could not resolve app data dir")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(dictations).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

/// Persist a finished recording: decode the base64 audio to
/// `dictations/<id>.<ext>` and index it in `dictations.json`.
pub fn save(
    app: &AppHandle,
    audio_base64: &str,
    mime: &str,
    text: &str,
    answer: Option<String>,
    duration_ms: Option<u64>,
) -> Result<Dictation, String> {
    let dir = audio_dir(app).ok_or("could not resolve app data dir")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_base64.trim())
        .map_err(|e| format!("invalid audio payload: {e}"))?;
    if bytes.is_empty() {
        return Err("empty audio payload".into());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let ext = ext_for_mime(mime);
    fs::write(dir.join(format!("{id}.{ext}")), &bytes).map_err(|e| e.to_string())?;

    let dictation = Dictation {
        id,
        text: text.trim().to_string(),
        answer,
        mime: mime.to_string(),
        created: Some(Local::now()),
        duration_ms,
    };
    let mut all = load_all(app);
    all.push(dictation.clone());
    save_all(app, &all)?;
    Ok(dictation)
}

/// List dictations, newest first. Optional `query` filters by substring.
pub fn list(app: &AppHandle, query: Option<&str>) -> Vec<Dictation> {
    let q = query.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let mut items: Vec<Dictation> = load_all(app)
        .into_iter()
        .filter(|d| q.is_empty() || d.text.to_lowercase().contains(&q))
        .collect();
    items.sort_by_key(|d| std::cmp::Reverse(d.created));
    items
}

pub fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut all = load_all(app);
    all.retain(|d| d.id != id);
    save_all(app, &all)?;
    // Best-effort removal of the recording file (any extension the mime
    // mapping could have produced).
    if let Some(dir) = audio_dir(app) {
        for ext in ["webm", "m4a", "wav", "ogg", "mp3"] {
            let _ = fs::remove_file(dir.join(format!("{id}.{ext}")));
        }
    }
    Ok(())
}

/// The recording as a `data:` URL the webview can play directly.
pub fn audio_data_url(app: &AppHandle, id: &str) -> Result<String, String> {
    // IDs are UUIDs; refuse anything else so the path can't be escaped.
    if !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err("invalid dictation id".into());
    }
    let meta = load_all(app)
        .into_iter()
        .find(|d| d.id == id)
        .ok_or("dictation not found")?;
    let dir = audio_dir(app).ok_or("could not resolve app data dir")?;
    let bytes = fs::read(dir.join(format!("{}.{}", meta.id, ext_for_mime(&meta.mime))))
        .map_err(|e| format!("recording missing: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mime = meta.mime.split(';').next().unwrap_or("").trim();
    if mime.is_empty() {
        Ok(format!("data:audio/webm;base64,{b64}"))
    } else {
        Ok(format!("data:{mime};base64,{b64}"))
    }
}