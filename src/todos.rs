use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::paths;
use crate::platform;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub text: String,
    /// When the todo/reminder is due (None = someday, shows in "today").
    #[serde(default)]
    pub due: Option<DateTime<Local>>,
    #[serde(default)]
    pub done: bool,
    /// True once the reminder notification has been fired for a due item.
    #[serde(default)]
    pub notified: bool,
    #[serde(default)]
    pub created: Option<DateTime<Local>>,
}

fn todos_path() -> Option<PathBuf> {
    paths::data_file("todos.json")
}

pub fn load_all() -> Vec<Todo> {
    match todos_path().and_then(|p| fs::read_to_string(p).ok()) {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => Vec::new(),
    }
}

pub fn save_all(todos: &[Todo]) -> Result<(), String> {
    let path = todos_path().ok_or("could not resolve app data dir")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(todos).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

/// Parse flexible due-date formats:
/// RFC3339, "YYYY-MM-DDTHH:MM", "YYYY-MM-DD HH:MM", "YYYY-MM-DD".
pub fn parse_due(raw: &str) -> Option<DateTime<Local>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Local));
    }
    for fmt in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(Local.from_local_datetime(&ndt).single()?);
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let ndt = d.and_hms_opt(9, 0, 0)?;
        return Some(Local.from_local_datetime(&ndt).single()?);
    }
    None
}

pub fn add(text: &str, due_raw: Option<&str>) -> Result<Todo, String> {
    let mut todos = load_all();
    let todo = Todo {
        id: uuid::Uuid::new_v4().to_string(),
        text: text.trim().to_string(),
        due: due_raw.and_then(parse_due),
        done: false,
        notified: false,
        created: Some(Local::now()),
    };
    todos.push(todo.clone());
    save_all(&todos)?;
    Ok(todo)
}

pub fn complete(id: &str) -> Result<(), String> {
    let mut todos = load_all();
    match todos.iter_mut().find(|t| t.id == id) {
        Some(t) => {
            t.done = true;
            save_all(&todos)
        }
        None => Err(format!("no todo with id {id}")),
    }
}

/// Check (done = true) or uncheck (done = false) a todo. Unchecking also
/// clears the notified flag so a reminder can fire again if it comes due.
pub fn set_done(id: &str, done: bool) -> Result<(), String> {
    let mut todos = load_all();
    match todos.iter_mut().find(|t| t.id == id) {
        Some(t) => {
            t.done = done;
            if !done {
                t.notified = false;
            }
            save_all(&todos)
        }
        None => Err(format!("no todo with id {id}")),
    }
}

pub fn delete(id: &str) -> Result<(), String> {
    let mut todos = load_all();
    todos.retain(|t| t.id != id);
    save_all(&todos)
}

/// scope: "today" | "tomorrow" | "all"
pub fn list(scope: &str) -> Vec<Todo> {
    let todos = load_all();
    let today = Local::now().date_naive();
    let tomorrow = today.succ_opt().unwrap_or(today);

    let mut out: Vec<Todo> = todos
        .into_iter()
        .filter(|t| match scope {
            "today" => !t.done && t.due.map(|d| d.date_naive() <= today).unwrap_or(true),
            "tomorrow" => {
                !t.done && t.due.map(|d| d.date_naive() == tomorrow).unwrap_or(false)
            }
            _ => true,
        })
        .collect();

    out.sort_by_key(|t| t.due);
    out
}

// ---------------------------------------------------------------------------
// Reminder scheduler (background thread)
// ---------------------------------------------------------------------------

/// Fire notifications for todos that just came due. Runs every 15s.
pub fn spawn_reminder_loop() {
    std::thread::spawn(|| loop {
        if let Err(e) = check_due() {
            eprintln!("reminder check failed: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(15));
    });
}

fn check_due() -> Result<(), String> {
    let mut todos = load_all();
    let now = Local::now();
    let mut changed = false;

    for todo in todos.iter_mut() {
        if !todo.done && !todo.notified {
            if let Some(due) = todo.due {
                if due <= now {
                    platform::notify("Supacast reminder", &todo.text);
                    todo.notified = true;
                    changed = true;
                }
            }
        }
    }

    if changed {
        save_all(&todos)?;
    }
    Ok(())
}
