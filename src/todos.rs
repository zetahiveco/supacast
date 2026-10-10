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
    /// When the last reminder notification for this todo was fired
    /// (None = never). Overdue, incomplete todos re-notify on an interval
    /// (Settings → notification frequency) until done. Replaces the old
    /// one-shot `notified` bool, which is ignored on read.
    #[serde(default)]
    pub notified_at: Option<DateTime<Local>>,
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
        notified_at: None,
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
/// clears the last-notified time so the reminder restarts from "notify
/// now" when it comes due again.
pub fn set_done(id: &str, done: bool) -> Result<(), String> {
    let mut todos = load_all();
    match todos.iter_mut().find(|t| t.id == id) {
        Some(t) => {
            t.done = done;
            if !done {
                t.notified_at = None;
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

/// Cron for due-todo notifications while the process is running (even with
/// the launcher window hidden — the app lives in the menu bar/tray). Ticks
/// every 15s: a todo past its due time is notified within seconds.
///
/// An overdue, incomplete todo re-notifies on the configured interval
/// (Settings → notification frequency, default 30 min) until it is checked
/// off. `None` means notify once and stop.
///
/// The first pass right after launch is a catch-up: reminders that came due
/// while the app was closed (process not running) are fired then — a few
/// individually, a big pile as one summary so reopening isn't spammed.
pub fn spawn_reminder_loop(shared: std::sync::Arc<crate::app::Shared>) {
    std::thread::spawn(move || {
        if let Err(e) = check_missed() {
            eprintln!("reminder catch-up failed: {e}");
        }
        loop {
            let repeat_min = shared
                .settings
                .lock()
                .unwrap()
                .remind_repeat_min;
            if let Err(e) = check_due(repeat_min) {
                eprintln!("reminder check failed: {e}");
            }
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    });
}

/// Number of missed reminders still sent one-by-one before batching into a
/// single summary notification (launch-time only).
const MISSED_INDIVIDUAL_MAX: usize = 3;

/// Startup catch-up: notify never-notified todos that became due while the
/// app was not running. One or two are sent individually (the body carries
/// the todo text); anything more is summarized into a single notification
/// pointing at the Todos view (which shows an "Overdue" group), so
/// reopening after a few days doesn't stack a dozen popups.
fn check_missed() -> Result<(), String> {
    let mut todos = load_all();
    let now = Local::now();

    let is_missed = |t: &Todo| {
        !t.done && t.notified_at.is_none() && t.due.map(|d| d <= now).unwrap_or(false)
    };
    let missed: Vec<String> = todos
        .iter()
        .filter(|t| is_missed(t))
        .map(|t| t.text.clone())
        .collect();
    if missed.is_empty() {
        return Ok(());
    }

    if missed.len() <= MISSED_INDIVIDUAL_MAX {
        for text in &missed {
            platform::notify("Supacast reminder (missed)", text);
        }
    } else {
        platform::notify(
            "Supacast reminders",
            &format!(
                "{} reminders came due while Supacast was closed — open Todos to review them.",
                missed.len()
            ),
        );
    }

    for todo in todos.iter_mut() {
        if is_missed(todo) {
            todo.notified_at = Some(now);
        }
    }
    save_all(&todos)
}

/// Periodic pass: notify todos that just came due, and re-notify overdue
/// incomplete todos whose repeat interval has elapsed since the last
/// notification. `repeat_min` is the Settings frequency (None = once).
fn check_due(repeat_min: Option<u64>) -> Result<(), String> {
    let mut todos = load_all();
    let now = Local::now();
    let mut changed = false;

    for todo in todos.iter_mut() {
        if todo.done {
            continue;
        }
        let Some(due) = todo.due else {
            continue;
        };
        if due > now {
            continue;
        }
        // Fire now if never notified, or if the repeat interval has
        // elapsed since the last notification (None = never repeat).
        let due_now = match todo.notified_at {
            None => true,
            Some(at) => match repeat_min {
                None => false,
                Some(mins) => now >= at + chrono::Duration::minutes(mins as i64),
            },
        };
        if due_now {
            platform::notify("Supacast reminder", &todo.text);
            todo.notified_at = Some(now);
            changed = true;
        }
    }

    if changed {
        save_all(&todos)?;
    }
    Ok(())
}
