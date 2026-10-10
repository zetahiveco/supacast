//! The launcher window: search bar + results / todos / notes / clipboard /
//! dictations / AI chat views. A direct port of the React `launcher.tsx`.

use chrono::{Local, TimeZone};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense};
use std::time::{Duration, Instant};

use crate::app::{App, ChatTurn, View};
use crate::events::DictateMode;
use crate::search::SearchResult;
use crate::ui::{ACCENT, BORDER, GREEN, PURPLE, RED, ROW_ACTIVE, ROW_HOVER, SUBTEXT, TEXT};

const ROW_H: f32 = 36.0;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    // Track real mouse movement (rows under a stationary cursor must not
    // steal the selection — same as the webview version).
    let moved = ui.input(|i| i.pointer.delta().length_sq() > 0.0);
    if moved {
        app.mouse_moved = true;
    }

    // While a note is being edited inline the search bar steps aside: the
    // editor owns the keyboard, so nothing must compete for focus.
    let editing_note = matches!(app.view, View::Notes { .. }) && app.editing_note.is_some();

    // -------------------------------------------------------------
    // Search row
    // -------------------------------------------------------------
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        let in_chat = app.view == View::Chat;
        let hint = if in_chat {
            if app.chat.is_empty() {
                "Ask Supacast…"
            } else {
                "Reply to Supacast…"
            }
        } else {
            "Search, or type a command…"
        };

        const SEARCH_ROW_H: f32 = 28.0;

        // Search icon, vertically centered in a fixed slot so it lines up
        // with the input text. While a search is in flight the spinner
        // takes over this slot (Spotlight-style): the row layout never
        // changes for progress, so the Clear button stays pinned.
        let (icon_rect, _) =
            ui.allocate_exact_size(egui::vec2(24.0, SEARCH_ROW_H), egui::Sense::hover());
        if app.searching {
            egui::Spinner::new()
                .size(16.0)
                .paint_at(ui, icon_rect.shrink2(egui::vec2(1.0, 4.0)));
        } else {
            crate::ui::draw_search(
                ui.painter(),
                icon_rect.shrink2(egui::vec2(1.0, 4.0)),
                crate::ui::TEXT,
            );
        }

        // Reserve room for the Clear button only — the input width must
        // not depend on whether a search is running.
        if editing_note {
            ui.add(egui::Label::new(
                egui::RichText::new("Editing note — Enter saves, Esc cancels")
                    .size(14.0)
                    .color(SUBTEXT),
            ));
        } else {
            let mut reserve = 14.0;
            if in_chat && !app.chat.is_empty() {
                reserve += 60.0; // Clear button
            }
            let available = ui.available_width() - reserve;
            let response = ui.add_sized(
                [available, SEARCH_ROW_H],
                egui::TextEdit::singleline(&mut app.query)
                    .hint_text(hint)
                    .font(egui::TextStyle::Heading)
                    .vertical_align(egui::Align::Center)
                    .frame(egui::Frame::NONE),
            );
            if app.need_focus {
                response.request_focus();
                app.need_focus = false;
            }

            // Clear pinned to the right edge of the row, always in the same
            // spot whether or not a search is running.
            if in_chat && !app.chat.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        app.clear_chat();
                    }
                });
            }
        }
    });

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);

    // -------------------------------------------------------------
    // Keyboard
    // -------------------------------------------------------------
    let (esc, up, down, enter, backspace) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Backspace),
        )
    });

    if app.view == View::Chat {
        if esc {
            app.hide_launcher(ui.ctx());
        } else if enter {
            ui.ctx().request_repaint();
            let q = app.query.trim().to_string();
            if q.is_empty() && app.dictate_thread {
                // Empty Enter in a dictated thread → back into dictation.
                app.query.clear();
                app.open_dictate(DictateMode::Supacast, ui.ctx());
            } else if app.chat.is_empty() && is_dictate_query(&q) {
                let mode = if q.contains("text") { DictateMode::Text } else { DictateMode::Supacast };
                app.query.clear();
                app.open_dictate(mode, ui.ctx());
            } else {
                app.send_chat(&q);
            }
        } else if backspace && app.query.is_empty() {
            // Empty input + Backspace leaves the chat; the thread is kept —
            // typing "ai" re-enters it.
            app.view = View::Search;
            app.need_focus = true;
        }
        draw_chat(app, ui);
        return;
    }

    // Inline note editing owns the keyboard: Enter saves, Esc cancels, the
    // caret moves with the arrows (no list navigation while typing).
    if let View::Notes { query } = &app.view {
        if app.editing_note.is_some() {
            let id = app.editing_note.clone().unwrap();
            let query = query.clone();
            if esc {
                app.editing_note = None;
                app.edit_note_text.clear();
            } else if enter {
                let text = std::mem::take(&mut app.edit_note_text);
                crate::notes::update(&id, &text).ok();
                app.editing_note = None;
                let keep = app.sel_note.clone();
                app.load_notes(&query);
                app.sel_note = keep;
                if let Some(idx) = app.notes.iter().position(|n| n.id == id) {
                    app.active = idx;
                }
            }
            app.need_focus = false;
            draw_notes(app, ui, &query);
            return;
        }
    }

    // Length of the active list (for arrow navigation).
    let list_len = match app.view {
        View::Search => suggestions(app).len() + app.results.len(),
        View::Todos => flat_todos(app).len(),
        View::Notes { .. } => app.notes.len(),
        View::Clipboard => app.clips.len(),
        View::Dictations => app.dictations.len(),
        View::Chat => 0,
    };

    if esc {
        app.hide_launcher(ui.ctx());
    } else if down && list_len > 0 {
        app.mouse_moved = false;
        app.scroll_pending = true;
        app.active = (app.active + 1).min(list_len - 1);
    } else if up && list_len > 0 {
        app.mouse_moved = false;
        app.scroll_pending = true;
        app.active = app.active.saturating_sub(1);
    } else if enter {
        ui.ctx().request_repaint();
        match &app.view {
            View::Search => {
                let q = app.query.trim().to_string();
                if q == "ai" {
                    app.query.clear();
                    app.chat.clear();
                    app.dictate_thread = false;
                    app.view = View::Chat;
                    app.need_focus = true;
                    return;
                }
                let all = [suggestions(app), app.results.clone()].concat();
                if let Some(item) = all.get(app.active) {
                    let item = item.clone();
                    if !handle_special(app, &item.path) {
                        open_and_hide(app, &item.path);
                    }
                }
            }
            View::Todos => {
                let flat = flat_todos(app);
                if let Some(t) = flat.get(app.active).cloned() {
                    crate::todos::set_done(&t.todo.id, !t.todo.done).ok();
                    app.load_todos();
                }
            }
            View::Notes { .. } => {
                // Enter on a collapsed note expands it; on an already
                // expanded note it starts an inline edit of the text.
                if let Some(n) = app.notes.get(app.active).cloned() {
                    if app.sel_note.as_deref() == Some(n.id.as_str()) {
                        app.editing_note = Some(n.id.clone());
                        app.edit_note_text = n.text.clone();
                    } else {
                        app.sel_note = Some(n.id);
                    }
                }
            }
            View::Clipboard => {
                if let Some(c) = app.clips.get(app.active).cloned() {
                    copy_clip_and_hide(app, &c.text);
                }
            }
            View::Dictations => {
                if let Some(d) = app.dictations.get(app.active).cloned() {
                    toggle_dictation(app, &d.id);
                }
            }
            View::Chat => {}
        }
    }

    // -------------------------------------------------------------
    // Views
    // -------------------------------------------------------------
    let view = app.view.clone();
    match view {
        View::Search => draw_search(app, ui),
        View::Todos => draw_todos(app, ui),
        View::Notes { ref query } => draw_notes(app, ui, query),
        View::Clipboard => draw_clipboard(app, ui),
        View::Dictations => draw_dictations(app, ui),
        View::Chat => {}
    }
}

// ---------------------------------------------------------------------------
// Search view
// ---------------------------------------------------------------------------

fn draw_search(app: &mut App, ui: &mut egui::Ui) {
    let all = [suggestions(app), app.results.clone()].concat();

    if all.is_empty() {
        let q = app.query.trim();
        if q.is_empty() {
            hint(ui, "Type to search • Try “todo”, “notes”, “clipboard history”, “dictations”, “note: …”");
        } else if !app.searching {
            hint(ui, &format!("No results for “{q}”"));
        } else {
            hint(ui, "Searching…");
        }
        return;
    }

    // Empty query → the list below is pending todos + the command
    // reference; label the sections.
    let empty_query = app.query.trim().is_empty();
    let scroll = egui::ScrollArea::vertical().auto_shrink(false);
    scroll.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut last_kind = "";
        for (i, item) in all.iter().enumerate() {
            // Section headers: "TODOS" above the first todo row, "COMMANDS"
            // above the first command row (only with an empty query).
            if empty_query && item.kind != last_kind {
                let header = if item.kind == "todo" { "TODOS" } else { "COMMANDS" };
                ui.add_space(8.0);
                ui.add(egui::Label::new(
                    egui::RichText::new(header).small().color(SUBTEXT),
                ));
                ui.add_space(2.0);
                last_kind = &item.kind;
            }
            let badge_color = match item.kind.as_str() {
                "todo" => ACCENT,
                "app" if item.path.starts_with("__") => PURPLE,
                "app" => ACCENT,
                "folder" => GREEN,
                _ => SUBTEXT,
            };
            let badge = match item.kind.as_str() {
                "todo" => "Todo",
                "app" if item.path.starts_with("__ask__") || item.path == "__chat__" => "AI",
                "app" if item.path.starts_with("__dictate") => "Mic",
                "app" if item.path.starts_with("__view_") => "Open",
                "app" if item.path.starts_with("__cmd_") => "Cmd",
                "app" => "App",
                "folder" => "Folder",
                _ => "File",
            };
            let (clicked, _) = result_row(ui, app, i, badge, badge_color, &item.title, &item.subtitle, false, None);
            if clicked {
                if !handle_special(app, &item.path) {
                    open_and_hide(app, &item.path);
                }
                break;
            }
        }
    });
}

/// Suggestions shown while typing in search view (local todo commands,
/// AI hand-off and dictate entry points — no AI needed). With an empty
/// query this becomes the launcher's browsable command reference.
fn suggestions(app: &App) -> Vec<SearchResult> {
    let mut out = Vec::new();
    let q = app.query.trim();
    if app.view != View::Search {
        return out;
    }
    // Empty search bar: incomplete todos first, then every built-in
    // command with a one-line description.
    if q.is_empty() {
        let mut out = pending_todo_rows(app);
        out.extend(builtin_commands());
        return out;
    }
    let lower = q.to_lowercase();

    if let Some(text) = lower.strip_prefix("add todo ") {
        if !text.is_empty() {
            out.push(SearchResult {
                title: format!("Add todo: “{}”", text),
                subtitle: "Press Enter to save — no AI needed".into(),
                path: format!("__add_todo__:{}", q[9..].trim()),
                kind: "app".into(),
            });
        }
    }
    if let Some((text, pretty, iso)) = parse_remind(q) {
        out.push(SearchResult {
            title: format!("Reminder: “{text}”"),
            subtitle: format!("Press Enter to save — notifies at {pretty}"),
            path: format!("__remind__:{text}|{iso}"),
            kind: "app".into(),
        });
    }
    let add_note = extract_after(&["add note", "note"], q);
    if let Some(text) = add_note {
        out.push(SearchResult {
            title: format!("Add note: “{text}”"),
            subtitle: "Press Enter to save — no AI needed".into(),
            path: format!("__add_note__:{text}"),
            kind: "app".into(),
        });
    }

    out.push(SearchResult {
        title: format!("Ask Supacast: “{q}”"),
        subtitle: "AI chat — todos, reminders, calendar".into(),
        path: "__ask__".into(),
        kind: "app".into(),
    });
    out.push(SearchResult {
        title: "Dictate (Text)".into(),
        subtitle: "Record & transcribe → clipboard".into(),
        path: "__dictate_text__".into(),
        kind: "app".into(),
    });
    out.push(SearchResult {
        title: "Dictate (Supacast)".into(),
        subtitle: "Record & ask the Supacast AI".into(),
        path: "__dictate_supacast__".into(),
        kind: "app".into(),
    });
    // "clear clipboard" — typed command to wipe the clip history.
    if lower.starts_with("clear") {
        let n = crate::clipboard_hist::get_history().len();
        out.push(SearchResult {
            title: "Clear clipboard history".into(),
            subtitle: format!(
                "Deletes all {n} clips — press Enter to confirm"
            ),
            path: "__clear_clipboard__".into(),
            kind: "app".into(),
        });
    }
    out
}

/// The launcher's built-in commands, shown below an empty search bar.
/// View commands jump straight to a section; "Cmd" rows are syntax
/// examples — selecting one pre-fills the query so you finish typing.
fn builtin_commands() -> Vec<SearchResult> {
    vec![
        SearchResult {
            title: "todo".into(),
            subtitle: "Show your todo list — toggle & delete inline".into(),
            path: "__view_todos__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "notes".into(),
            subtitle: "Browse notes — “notes wifi” filters them".into(),
            path: "__view_notes__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "clipboard history".into(),
            subtitle: "Recent clips — Enter copies again".into(),
            path: "__view_clipboard__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "clear clipboard".into(),
            subtitle: "Delete the entire clipboard history".into(),
            path: "__clear_clipboard__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "dictations".into(),
            subtitle: "Transcript history with inline playback".into(),
            path: "__view_dictations__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "add todo <text>".into(),
            subtitle: "Quick-add a todo — no AI needed".into(),
            path: "__cmd_add_todo__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "note: <text>".into(),
            subtitle: "Quick-save a note — no AI needed".into(),
            path: "__cmd_add_note__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "remind me to <text> at 5pm".into(),
            subtitle: "Time-based reminder — notifies when due".into(),
            path: "__cmd_remind__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "ai".into(),
            subtitle: "Ask Supacast — todos, reminders, calendar".into(),
            path: "__chat__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "dictate (text)".into(),
            subtitle: "Record & transcribe → clipboard".into(),
            path: "__dictate_text__".into(),
            kind: "app".into(),
        },
        SearchResult {
            title: "dictate (supacast)".into(),
            subtitle: "Record & ask the Supacast AI".into(),
            path: "__dictate_supacast__".into(),
            kind: "app".into(),
        },
    ]
}

/// Incomplete todos shown above the command list when the search bar is
/// empty — soonest-due first, undated last. Capped so a long list can't
/// push the command reference out of reach; the overflow becomes a single
/// "open Todos" row.
fn pending_todo_rows(app: &App) -> Vec<SearchResult> {
    const MAX_SHOWN: usize = 5;

    let mut pending: Vec<&crate::todos::Todo> = app.todos.iter().filter(|t| !t.done).collect();
    // Dated todos first (soonest due at the top), undated after. The sort
    // is stable so ties keep their saved order.
    pending.sort_by_key(|t| t.due.is_none());

    let total = pending.len();
    let mut rows: Vec<SearchResult> = pending
        .into_iter()
        .take(MAX_SHOWN)
        .map(|t| SearchResult {
            title: t.text.clone(),
            subtitle: {
                let s = crate::app::fmt_due(t.due);
                if s.is_empty() { "No due date".into() } else { s }
            },
            path: "__view_todos__".into(),
            kind: "todo".into(),
        })
        .collect();

    let hidden = total - rows.len();
    if hidden > 0 {
        rows.push(SearchResult {
            title: format!("+{hidden} more todo{plural}", plural = if hidden == 1 { "" } else { "s" }),
            subtitle: "Open Todos to see everything".into(),
            path: "__view_todos__".into(),
            kind: "todo".into(),
        });
    }
    rows
}

/// Case-insensitively strip a prefix that must be followed by whitespace,
/// then remove an optional ": " / "that " — for "note: buy milk" style
/// commands. Returns the (original-case) remainder.
fn extract_after(prefixes: &[&str], q: &str) -> Option<String> {
    let l = q.to_lowercase();
    for p in prefixes {
        if let Some(rest) = l.strip_prefix(p) {
            if !rest.starts_with(' ') {
                continue; // e.g. "notepad" must not match "note"
            }
            let orig = &q[p.len()..];
            let t = orig.trim_start().trim_start_matches(':').trim_start();
            let t = t.strip_prefix("that ").unwrap_or(t).trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// "remind me to X at 5[PM]" / "remind me X by 17:30" → (text, pretty, iso).
fn parse_remind(q: &str) -> Option<(String, String, String)> {    let lower = q.to_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("remind me to ") {
        r
    } else if let Some(r) = lower.strip_prefix("remind me ") {
        r
    } else {
        return None;
    };
    let idx = rest.rfind(" at ").or_else(|| rest.rfind(" by "))?;
    let text = rest[..idx].trim().to_string();
    let time_raw = rest[idx + 4..].trim();

    // Strip an am/pm suffix.
    let (time_part, is_pm, is_am) = if let Some(t) = time_raw.strip_suffix("pm") {
        (t.trim(), true, false)
    } else if let Some(t) = time_raw.strip_suffix("am") {
        (t.trim(), false, true)
    } else {
        (time_raw, false, false)
    };

    let (h_raw, m_raw) = match time_part.split_once(':') {
        Some((h, m)) => (h, m),
        None => (time_part, ""),
    };
    let mut h: i64 = h_raw.parse().ok()?;
    let m: i64 = if m_raw.is_empty() { 0 } else { m_raw.parse().ok()? };
    if !(0..=24).contains(&h) || !(0..=59).contains(&m) {
        return None;
    }
    if is_pm && h < 12 {
        h += 12;
    }
    if is_am && h == 12 {
        h = 0;
    }

    let now = Local::now();
    let mut due = now
        .date_naive()
        .and_hms_opt(h as u32, m as u32, 0)
        .and_then(|t| Local.from_local_datetime(&t).single())?;
    if due <= now {
        due += chrono::Duration::days(1); // "at 5pm" tomorrow if past
    }

    Some((
        text,
        due.format("%-I:%M %p").to_string(),
        due.to_rfc3339(),
    ))
}

/// Special suggestion paths: add todos/notes/reminders locally, jump to
/// AI chat or dictate mode. Returns true when the path was special.
fn handle_special(app: &mut App, path: &str) -> bool {
    if let Some(text) = path.strip_prefix("__add_todo__:") {
        let text = text.to_string();
        crate::todos::add(&text, None).ok();
        app.query.clear();
        app.view = View::Todos;
        app.load_todos();
        app.need_focus = true;
        return true;
    }
    if let Some(text) = path.strip_prefix("__add_note__:") {
        let text = text.to_string();
        crate::notes::add(&text).ok();
        app.query.clear();
        app.view = View::Notes { query: String::new() };
        app.load_notes("");
        app.need_focus = true;
        return true;
    }
    if let Some(rest) = path.strip_prefix("__remind__:") {
        let (text, iso) = rest.split_once('|').unwrap_or((rest, ""));
        let text = text.to_string();
        crate::todos::add(&text, Some(iso)).ok();
        app.query.clear();
        app.view = View::Todos;
        app.load_todos();
        app.need_focus = true;
        return true;
    }
    if path == "__ask__" {
        let q = app.query.trim().to_string();
        app.send_chat(&q);
        return true;
    }
    if path == "__chat__" {
        // "ai" from the empty-state command list: enter chat with no thread.
        app.query.clear();
        app.chat.clear();
        app.dictate_thread = false;
        app.view = View::Chat;
        app.need_focus = true;
        return true;
    }
    match path {
        "__view_todos__" => {
            app.view = View::Todos;
            app.load_todos();
            return true;
        }
        "__view_notes__" => {
            app.view = View::Notes { query: String::new() };
            app.load_notes("");
            return true;
        }
        "__view_clipboard__" => {
            app.view = View::Clipboard;
            app.clips = crate::clipboard_hist::get_history();
            return true;
        }
        "__clear_clipboard__" => {
            crate::clipboard_hist::clear_history();
            // Show the emptied view as inline feedback instead of a
            // notification banner.
            app.query.clear();
            app.view = View::Clipboard;
            app.clips = Vec::new();
            app.need_focus = true;
            return true;
        }
        "__view_dictations__" => {
            app.view = View::Dictations;
            app.load_dictations();
            return true;
        }
        // Syntax examples: pre-fill the query so the user finishes typing.
        "__cmd_add_todo__" => {
            app.query = "add todo ".into();
            return true;
        }
        "__cmd_add_note__" => {
            app.query = "note: ".into();
            return true;
        }
        "__cmd_remind__" => {
            app.query = "remind me to ".into();
            return true;
        }
        "__dictate_text__" => {
            if let Some(ctx) = app_ctx(app).cloned() {
                app.open_dictate(DictateMode::Text, &ctx);
            }
            return true;
        }
        "__dictate_supacast__" => {
            if let Some(ctx) = app_ctx(app).cloned() {
                app.open_dictate(DictateMode::Supacast, &ctx);
            }
            return true;
        }
        _ => {}
    }
    false
}

fn app_ctx(app: &App) -> Option<&egui::Context> {
    app.shared.ctx.get()
}

fn open_and_hide(app: &mut App, path: &str) {
    if crate::search::open_path(path).is_ok() {
        if let Some(ctx) = app_ctx(app).cloned() {
            app.hide_launcher(&ctx);
        }
    }
}

fn copy_clip_and_hide(app: &mut App, text: &str) {
    if crate::clipboard_hist::copy_to_clipboard(text).is_ok() {
        if let Some(ctx) = app_ctx(app).cloned() {
            app.hide_launcher(&ctx);
        }
    }
}

// ---------------------------------------------------------------------------
// Todos view
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FlatTodo {
    label: Option<String>,
    todo: crate::todos::Todo,
}

/// Date-wise grouping (Overdue / Today / Tomorrow / Upcoming / Someday /
/// Completed) flattened for keyboard navigation.
fn flat_todos(app: &App) -> Vec<FlatTodo> {
    let mut groups: Vec<(&str, Vec<crate::todos::Todo>)> = vec![
        ("Overdue", vec![]),
        ("Today", vec![]),
        ("Tomorrow", vec![]),
        ("Upcoming", vec![]),
        ("Someday", vec![]),
        ("Completed", vec![]),
    ];

    let today = Local::now().date_naive();
    let tomorrow = today.succ_opt().unwrap_or(today);
    for t in &app.todos {
        if t.done {
            groups[5].1.push(t.clone());
        } else {
            match t.due {
                None => groups[4].1.push(t.clone()),
                Some(due) => {
                    let d = due.date_naive();
                    if d < today {
                        groups[0].1.push(t.clone());
                    } else if d == today {
                        groups[1].1.push(t.clone());
                    } else if d == tomorrow {
                        groups[2].1.push(t.clone());
                    } else {
                        groups[3].1.push(t.clone());
                    }
                }
            }
        }
    }
    for g in groups.iter_mut() {
        g.1.sort_by_key(|t| t.due);
    }
    let mut out = Vec::new();
    for (label, items) in groups {
        if items.is_empty() {
            continue;
        }
        for t in items {
            out.push(FlatTodo { label: Some(label.to_string()), todo: t });
        }
        out.last_mut().unwrap().label = Some(label.to_string());
    }
    // Only the first item of each group carries the label.
    let mut seen = std::collections::HashSet::new();
    for item in out.iter_mut() {
        if let Some(l) = &item.label {
            if !seen.insert(l.clone()) {
                item.label = None;
            }
        }
    }
    out
}

fn draw_todos(app: &mut App, ui: &mut egui::Ui) {
    let flat = flat_todos(app);
    if flat.is_empty() {
        hint(ui, "No todos yet — try “add todo …” or ask the AI");
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, item) in flat.iter().enumerate() {
            if let Some(label) = &item.label {
                ui.add_space(6.0);
                ui.add(egui::Label::new(
                    egui::RichText::new(label.to_uppercase()).small().color(SUBTEXT),
                ));
            }
            let t = &item.todo;
            let sub = crate::app::fmt_due(t.due);
            let (clicked, delete) =
                result_row(ui, app, i, "", ACCENT, &t.text, &sub, true, Some(t.done));
            if delete {
                crate::todos::delete(&t.id).ok();
                app.load_todos();
                break;
            }
            if clicked {
                crate::todos::set_done(&t.id, !t.done).ok();
                app.load_todos();
                break;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Notes view
// ---------------------------------------------------------------------------

fn draw_notes(app: &mut App, ui: &mut egui::Ui, query: &str) {
    if app.notes.is_empty() {
        hint(ui, &if query.is_empty() {
            "No notes yet — try “note: buy milk” or ask the AI".to_string()
        } else {
            format!("No notes matching “{query}”")
        });
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, n) in app.notes.clone().iter().enumerate() {
            let selected = app.sel_note.as_deref() == Some(n.id.as_str());
            let (clicked, delete) = result_row(
                ui,
                app,
                i,
                "Note",
                ACCENT,
                &n.text,
                &crate::app::fmt_due(n.created),
                true,
                None,
            );
            if delete {
                crate::notes::delete(&n.id).ok();
                let q = query.to_string();
                app.sel_note = app.sel_note.take().filter(|id| id != &n.id);
                app.load_notes(&q);
                break;
            }
            if clicked {
                if selected {
                    // Collapsing a note also cancels its inline edit.
                    app.editing_note = None;
                    app.edit_note_text.clear();
                }
                app.sel_note = if selected { None } else { Some(n.id.clone()) };
                break;
            }
            if selected {
                // Pin the expanded panel to exactly the row width (same
                // trick as the dictations panel): claim the content width
                // (row width minus the frame's 8px margins on each side)
                // as both min and max, so the fill always spans the full
                // row — never narrower (content wrap) or wider (overflow).
                let row_w = ui.available_width();
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_min_width(row_w - 16.0);
                        ui.set_max_width(row_w - 16.0);
                        if app.editing_note.as_deref() == Some(n.id.as_str()) {
                            // Inline editor, transparent frame so it blends
                            // with the expanded panel background.
                            let editor = ui.add(
                                egui::TextEdit::multiline(&mut app.edit_note_text)
                                    .font(egui::FontId::proportional(13.5))
                                    .desired_width(ui.available_width())
                                    .frame(egui::Frame::NONE)
                                    .return_key(egui::KeyboardShortcut::new(
                                        egui::Modifiers::SHIFT,
                                        egui::Key::Enter,
                                    )),
                            );
                            if !editor.has_focus() {
                                editor.request_focus();
                            }
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(
                                egui::RichText::new(
                                    "Enter to save • Shift+Enter for a new line • Esc to cancel",
                                )
                                .small()
                                .color(SUBTEXT),
                            ));
                        } else {
                            // Full text + copy action. Copying keeps the
                            // launcher open: the button reads "Copied" for
                            // 2s, then reverts to "Copy".
                            ui.add(egui::Label::new(
                                egui::RichText::new(&n.text).size(13.5).color(TEXT),
                            ).wrap());
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let copied_until = app
                                    .copied_note
                                    .as_ref()
                                    .filter(|(id, _)| id == n.id.as_str())
                                    .map(|(_, at)| *at + Duration::from_secs(2));
                                let copied = copied_until
                                    .is_some_and(|until| Instant::now() < until);
                                let btn = ui.add(egui::Button::new(
                                    egui::RichText::new(if copied { "Copied ✓" } else { "Copy" })
                                        .color(if copied { GREEN } else { TEXT }),
                                ));
                                // Keep repainting until the label reverts.
                                if let Some(until) = copied_until.filter(|_| copied) {
                                    ui.ctx().request_repaint_after(
                                        until.saturating_duration_since(Instant::now()),
                                    );
                                }
                                if btn.clicked()
                                    && crate::clipboard_hist::copy_to_clipboard(&n.text).is_ok()
                                {
                                    app.copied_note = Some((n.id.clone(), Instant::now()));
                                }
                            });
                        }
                    });
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Clipboard view
// ---------------------------------------------------------------------------

fn draw_clipboard(app: &mut App, ui: &mut egui::Ui) {
    if app.clips.is_empty() {
        hint(ui, "Clipboard history is empty");
        return;
    }
    // Header: clip count on the left, Clear all on the right.
    let n = app.clips.len();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(format!("{n} clip{} — Enter copies", if n == 1 { "" } else { "s" }))
                    .size(12.5)
                    .color(SUBTEXT),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn = egui::Button::new(
                egui::RichText::new("Clear all").size(12.0).color(RED),
            )
            .fill(egui::Color32::from_rgba_unmultiplied_const(255, 90, 90, 22))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied_const(255, 90, 90, 60)))
            .corner_radius(0.0)
            .min_size(egui::vec2(70.0, 24.0));
            if ui.add(btn).clicked() {
                crate::clipboard_hist::clear_history();
                app.clips = Vec::new();
                app.active = 0;
            }
        });
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, c) in app.clips.clone().iter().enumerate() {
            let preview: String = c.text.chars().take(120).collect();
            let (clicked, _) = result_row(ui, app, i, "Clip", ACCENT, &preview, &crate::app::fmt_due(Some(c.at)), false, None);
            if clicked {
                copy_clip_and_hide(app, &c.text);
                break;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Dictations view
// ---------------------------------------------------------------------------

fn toggle_dictation(app: &mut App, id: &str) {
    if app.sel_dict.as_deref() == Some(id) {
        app.sel_dict = None;
        app.stop_dictation(id);
        return;
    }
    app.sel_dict = Some(id.to_string());
    // Pause whatever was playing; it keeps its position for later resume.
    app.pause_playbacks_except("");
}

fn draw_dictations(app: &mut App, ui: &mut egui::Ui) {
    if app.dictations.is_empty() {
        hint(ui, "No dictations yet — hold Enter on the dictate ring");
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, d) in app.dictations.clone().iter().enumerate() {
            let selected = app.sel_dict.as_deref() == Some(d.id.as_str());
            let dur = fmt_dur(d.duration_ms);
            let sub = [crate::app::fmt_due(d.created), dur]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" • ");
            let (clicked, delete) = result_row(
                ui,
                app,
                i,
                "Mic",
                PURPLE,
                &d.text,
                &sub,
                true,
                None,
            );
            if delete {
                crate::dictations::delete(&d.id).ok();
                app.stop_dictation(&d.id);
                if app.sel_dict.as_deref() == Some(d.id.as_str()) {
                    app.sel_dict = None;
                }
                app.load_dictations();
                break;
            }
            if clicked {
                toggle_dictation(app, &d.id);
                break;
            }
            if selected {
                // Pin the expanded panel to exactly the row width: claim
                // the content width (row width minus the frame's 8px
                // margins on each side) as both min and max, so the fill
                // always spans the full row — never narrower (content
                // wrap) or wider (overflow).
                let row_w = ui.available_width();
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_min_width(row_w - 16.0);
                        ui.set_max_width(row_w - 16.0);
                        ui.add(egui::Label::new(
                            egui::RichText::new(&d.text).size(13.0).color(TEXT),
                        ).wrap());
                        if let Some(answer) = &d.answer {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(
                                egui::RichText::new(answer).size(12.0).color(SUBTEXT),
                            ).wrap());
                        }
                        ui.add_space(4.0);
                        draw_player(app, ui, &d.id, d.duration_ms);
                    });
            }
        }
    });
}

/// Inline audio player for a dictation: a play/pause button, clickable
/// seek bar (with progress fill + knob) and a time readout. Vector icons
/// throughout — no font glyphs.
fn draw_player(app: &mut App, ui: &mut egui::Ui, id: &str, duration_ms: Option<u64>) {
    // Snapshot the handle state first (position, duration, paused) so the
    // mutable `app` is free for the click handlers below.
    let handle = app.playback_handle(id);
    let active = handle.is_some();
    let paused = handle.map(|h| h.is_paused()).unwrap_or(false);
    let (pos_s, dur_s) = match handle {
        Some(h) => (h.position_secs() as f32, h.duration_secs() as f32),
        None => (0.0, duration_ms.unwrap_or(0) as f32 / 1000.0),
    };

    // Keep the progress bar animating while this player is active.
    if active && !paused {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    ui.horizontal(|ui| {
        // ---- Play / pause button (single button; no stop) ----
        let (btn_rect, btn_resp) =
            ui.allocate_exact_size(egui::vec2(24.0, 22.0), egui::Sense::click());
        let p = ui.painter();
        if btn_resp.hovered() {
            p.rect_filled(btn_rect, 0.0, ROW_ACTIVE); // sharp
        }
        if active && !paused {
            crate::ui::draw_pause(p, btn_rect, TEXT);
        } else {
            crate::ui::draw_play(p, btn_rect, TEXT);
        }
        if btn_resp.clicked() {
            if active {
                if let Some(h) = app.playback_handle(id) {
                    h.set_paused(!paused);
                }
            } else {
                play_dictation(app, id);
            }
        }

        // ---- Seek bar ----
        let time_w = 74.0;
        let bar_w = (ui.available_width() - time_w - 8.0).max(40.0);
        let (bar_rect, bar_resp) =
            ui.allocate_exact_size(egui::vec2(bar_w, 16.0), egui::Sense::click_and_drag());
        let p = ui.painter();
        let track = egui::Rect::from_center_size(bar_rect.center(), egui::vec2(bar_w - 4.0, 4.0));
        p.rect_filled(track, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30));
        let frac = if dur_s > 0.0 { (pos_s / dur_s).clamp(0.0, 1.0) } else { 0.0 };
        if frac > 0.0 {
            let fill = egui::Rect::from_min_size(track.min, egui::vec2(track.width() * frac, 4.0));
            p.rect_filled(fill, 0.0, ACCENT);
        }
        let knob_x = track.min.x + track.width() * frac;
        p.circle_filled(
            egui::pos2(knob_x, track.center().y),
            5.0,
            if active { ACCENT } else { SUBTEXT },
        );

        // Click/drag on the bar seeks (only while this dictation is loaded).
        if active && (bar_resp.dragged() || bar_resp.clicked()) {
            if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                let f = ((pointer.x - track.min.x) / track.width()).clamp(0.0, 1.0);
                if let Some(h) = app.playback_handle(id) {
                    h.seek_secs(f as f64 * h.duration_secs());
                }
            }
        }

        // ---- Time readout ----
        p.text(
            egui::pos2(bar_rect.max.x + 8.0, bar_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &format!("{} / {}", fmt_secs(pos_s), fmt_secs(dur_s)),
            egui::FontId::proportional(11.0),
            SUBTEXT,
        );
    });
}

/// `12.0` -> `"0:12"`.
fn fmt_secs(secs: f32) -> String {
    let s = secs.max(0.0).round() as usize;
    format!("{}:{:02}", s / 60, s % 60)
}

fn play_dictation(app: &mut App, id: &str) {
    // Pause whatever else is playing — it keeps its position and resumes
    // when picked again; only one dictation plays at a time.
    app.pause_playbacks_except(id);
    // Already loaded (paused): just resume from where it was.
    if app.playback_handle(id).is_some() {
        if let Some(h) = app.playback_handle(id) {
            h.set_paused(false);
        }
        return;
    }
    match crate::dictations::load_audio_bytes(id) {
        Ok((bytes, _mime)) => {
            // wav (this app) and m4a/mp3/ogg (old Tauri app) all decode
            // here — everything plays inline in the player.
            match crate::audio::decode_audio(&bytes) {
                Ok((samples, rate)) => match crate::audio::play_samples(samples, rate) {
                    Ok(handle) => app.playbacks.push((id.to_string(), handle)),
                    Err(e) => crate::platform::notify("Supacast", &format!("Playback failed: {e}")),
                },
                Err(e) => crate::platform::notify("Supacast", &format!("Playback failed: {e}")),
            }
        }
        Err(e) => crate::platform::notify("Supacast", &format!("Recording missing: {e}")),
    }
}

fn fmt_dur(ms: Option<u64>) -> String {
    let ms = match ms {
        Some(ms) if ms > 0 => ms,
        _ => return String::new(),
    };
    let total = (ms / 1000) as usize;
    format!("{}:{:02}", total / 60, total % 60)
}

fn is_dictate_query(q: &str) -> bool {
    let l = q.trim().to_lowercase();
    l.starts_with("dictate")
}

// ---------------------------------------------------------------------------
// Chat view
// ---------------------------------------------------------------------------

fn draw_chat(app: &mut App, ui: &mut egui::Ui) {
    if app.chat.is_empty() {
        let q = app.query.trim().to_lowercase();
        let hint_text = if is_dictate_query(&q) {
            if q.contains("text") {
                "Press Enter to start Dictate (Text)…"
            } else {
                "Press Enter to start Dictate (Supacast)…"
            }
        } else {
            "Ask Supacast — type a question and press Enter • Backspace to go back"
        };
        ui.add_space(20.0);
        hint(ui, hint_text);
        return;
    }

    let mut chat = std::mem::take(&mut app.chat);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for turn in chat.iter_mut() {
                draw_turn(ui, turn);
            }
        });
    app.chat = chat;
}

/// One chat turn, ported from the original app.css `.chat-turn` styles:
/// the user message is a full-width strip with a separator line under it
/// and the assistant reply is plain text — no bubbles anywhere.
fn draw_turn(ui: &mut egui::Ui, turn: &mut ChatTurn) {
    if turn.role == "user" {
        // User message: a clearly visible full-width grey strip with the
        // small "You" label on top — sharp corners,
        // always edge-to-edge so it never reads as a chat bubble.
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 30))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width()); // strip = full row width
                ui.add(egui::Label::new(
                    egui::RichText::new("You").small().color(SUBTEXT),
                ));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&turn.content)
                            .size(13.5)
                            .strong()
                            .color(TEXT),
                    )
                    .wrap(),
                );
            });
        // The separator under the user message (app.css `border-bottom`).
        let (line_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 1.0),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(line_rect, 0.0, BORDER);
        ui.add_space(6.0);
        return;
    }

    // Assistant message: same full-width square strip as the user turn,
    // but with a subtle accent-purple wash so the roles read apart.
    let strip = if turn.error {
        Color32::from_rgba_unmultiplied(255, 123, 123, 22) // reddish for errors
    } else {
        Color32::from_rgba_unmultiplied(124, 92, 255, 20) // accent wash
    };
    egui::Frame::new()
        .fill(strip)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width()); // square, edge-to-edge
            ui.add(egui::Label::new(
                egui::RichText::new("Assistant").small().color(SUBTEXT),
            ));

            // Tool chips.
            if !turn.tools.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for t in &turn.tools {
                        ui.add(egui::Label::new(
                            egui::RichText::new(format!("🔧 {t}")).small().color(SUBTEXT),
                        ));
                    }
                });
                ui.add_space(2.0);
            }

            // Progress while waiting for the first byte.
            if turn.streaming && turn.content.is_empty() {
                ui.horizontal(|ui| {
                    if turn.waiting {
                        ui.spinner();
                        ui.add(egui::Label::new(
                            egui::RichText::new("Connecting to model…").color(SUBTEXT),
                        ));
                    } else {
                        ui.add(egui::Label::new(
                            egui::RichText::new("Thinking…").color(SUBTEXT),
                        ));
                    }
                    if let Some(t) = turn.started_at {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(
                                egui::RichText::new(format!("{}s", t.elapsed().as_secs()))
                                    .small()
                                    .color(SUBTEXT),
                            ));
                        });
                    }
                });
            }

            // Thinking trace.
            if !turn.thinking.trim().is_empty() {
                egui::CollapsingHeader::new(egui::RichText::new("Thinking").small().color(SUBTEXT))
                    .default_open(turn.streaming && turn.content.is_empty())
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(
                            egui::RichText::new(&turn.thinking).small().color(SUBTEXT),
                        ).wrap());
                    });
            }

            let mut text = turn.content.clone();
            if turn.streaming && !turn.content.is_empty() {
                text.push('▌');
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .size(13.5)
                        .color(if turn.error { RED } else { TEXT }),
                )
                .wrap(),
            );
        });
    ui.add_space(6.0);
}

// ---------------------------------------------------------------------------
// Row widget (shared by every list view)
// ---------------------------------------------------------------------------

/// One list row: [badge] title  subtitle  [✕]. Returns (row_clicked,
/// delete_clicked). Handles hover-select (only after real mouse movement)
/// and keyboard selection highlight.
#[allow(clippy::too_many_arguments)]
fn result_row(
    ui: &mut egui::Ui,
    app: &mut App,
    i: usize,
    badge: &str,
    badge_color: Color32,
    title: &str,
    subtitle: &str,
    deletable: bool,
    checked: Option<bool>,
) -> (bool, bool) {
    let active = i == app.active;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW_H), Sense::click());

    let hovered = resp.hovered() && app.mouse_moved;
    if hovered && !active {
        app.active = i;
    }
    // Keyboard selection = purple strip (app.css --panel-active); mouse
    // hover is a subtler white wash.
    let bg = if active {
        ROW_ACTIVE
    } else if hovered {
        ROW_HOVER
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg); // sharp row highlight
    }

    let mut x = rect.min.x + 12.0;
    if let Some(is_checked) = checked {
        // Painted checkbox (vector — glyph fonts render as squares).
        let cb = egui::Rect::from_min_size(
            Pos2::new(x, rect.center().y - 7.0),
            egui::vec2(14.0, 14.0),
        );
        let p = ui.painter();
        if is_checked {
            p.rect_filled(cb, 0.0, ACCENT);
            crate::ui::draw_check(p, cb, crate::ui::BG, 2.0);
        } else {
            p.rect_stroke(cb, 0.0, egui::Stroke::new(1.5, SUBTEXT), egui::StrokeKind::Inside);
        }
        x += 22.0;
    }
    if !badge.is_empty() {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            badge,
            FontId::proportional(11.0),
            badge_color,
        );
        x += badge.len() as f32 * 7.0 + 10.0;
    }

    // Title + dimmed subtitle on one line, truncated to fit.
    let title_color = if checked == Some(true) { SUBTEXT } else { TEXT };
    let del_w = if deletable { 30.0 } else { 0.0 };
    let max_w = rect.max.x - del_w - 8.0 - x;
    let galley = {
        let mut job = egui::text::LayoutJob {
            wrap: egui::text::TextWrapping {
                max_width: max_w,
                max_rows: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        job.append(
            title,
            0.0,
            egui::TextFormat::simple(FontId::proportional(14.0), title_color),
        );
        if !subtitle.is_empty() {
            job.append(
                &format!("   {subtitle}"),
                0.0,
                egui::TextFormat::simple(FontId::proportional(11.0), SUBTEXT),
            );
        }
        ui.painter().layout_job(job)
    };
    ui.painter().galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );

    // Delete button (a small ✕ at the trailing edge).
    let mut delete_clicked = false;
    if deletable {
        let del_rect = Rect::from_min_size(
            Pos2::new(rect.max.x - 28.0, rect.min.y + 4.0),
            egui::vec2(24.0, ROW_H - 8.0),
        );
        let del_resp = ui.interact(del_rect, ui.id().with(("del", i)), Sense::click());
        if del_resp.hovered() {
            ui.painter().rect_filled(del_rect, 0.0, Color32::from_rgba_unmultiplied(235, 110, 110, 40));
        }
        crate::ui::draw_x(ui.painter(), del_rect.center(), 5.0, SUBTEXT, 1.8);
        delete_clicked = del_resp.clicked();
    }

    // Keyboard scroll-into-view.
    if active && app.scroll_pending {
        resp.scroll_to_me(Some(egui::Align::Center));
    }

    (!delete_clicked && resp.clicked(), delete_clicked)
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(14.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Label::new(
            egui::RichText::new(text).size(13.0).color(SUBTEXT),
        ).wrap());
    });
}
