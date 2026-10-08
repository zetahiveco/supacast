mod ai;
mod calendar;
mod clipboard_hist;
mod dictations;
mod notes;
mod paste_focus;
mod search;
mod settings;
mod todos;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

// ---------------------------------------------------------------------------
// Commands (called from the React frontend)
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> settings::Settings {
    settings::load(&app)
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    shortcut: String,
    openai_api_key: String,
    chat_base_url: Option<String>,
    chat_model: Option<String>,
) -> Result<settings::Settings, String> {
    let mut current = settings::load(&app);
    let shortcut = normalize_shortcut(&shortcut);

    // Only re-register the global shortcut when it actually changed.
    if shortcut != current.shortcut {
        let parsed: Shortcut = shortcut
            .parse()
            .map_err(|_| format!("invalid shortcut: {shortcut}"))?;
        app.global_shortcut().unregister_all().map_err(|e| e.to_string())?;
        app.global_shortcut()
            .on_shortcut(parsed, |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    toggle_launcher(app);
                }
            })
            .map_err(|e| format!("could not register shortcut {shortcut}: {e}"))?;
    }

    current.shortcut = shortcut;
    current.openai_api_key = openai_api_key;
    if let Some(url) = chat_base_url {
        current.chat_base_url = url.trim().trim_end_matches('/').to_string();
    }
    if let Some(model) = chat_model {
        current.chat_model = model.trim().to_string();
    }
    settings::save(&app, &current)?;
    Ok(current)
}

#[tauri::command]
async fn search_apps(query: String) -> Vec<search::SearchResult> {
    // Async so this runs on a worker thread instead of blocking the main
    // thread (and the whole webview) while walking app directories.
    search::search_apps(&query)
}

#[tauri::command]
async fn search_files(query: String) -> Vec<search::SearchResult> {
    search::search_files(&query)
}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    search::open_path(&path)
}

#[tauri::command]
fn hide_launcher(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn open_settings(app: tauri::AppHandle) {
    show_settings(&app);
}

// --- Todos / reminders ---

#[tauri::command]
fn add_todo(
    app: tauri::AppHandle,
    text: String,
    due: Option<String>,
) -> Result<todos::Todo, String> {
    todos::add(&app, &text, due.as_deref())
}

#[tauri::command]
fn list_todos(app: tauri::AppHandle, scope: String) -> Vec<todos::Todo> {
    todos::list(&app, &scope)
}

#[tauri::command]
fn complete_todo(app: tauri::AppHandle, id: String) -> Result<(), String> {
    todos::complete(&app, &id)
}

/// Check or uncheck a todo from the list UI.
#[tauri::command]
fn set_todo_done(app: tauri::AppHandle, id: String, done: bool) -> Result<(), String> {
    todos::set_done(&app, &id, done)
}

#[tauri::command]
fn delete_todo(app: tauri::AppHandle, id: String) -> Result<(), String> {
    todos::delete(&app, &id)
}

// --- Notes ---

#[tauri::command]
fn add_note(app: tauri::AppHandle, text: String) -> Result<notes::Note, String> {
    notes::add(&app, &text)
}

#[tauri::command]
fn list_notes(app: tauri::AppHandle, query: Option<String>) -> Vec<notes::Note> {
    notes::list(&app, query.as_deref())
}

#[tauri::command]
fn delete_note(app: tauri::AppHandle, id: String) -> Result<(), String> {
    notes::delete(&app, &id)
}

// --- Dictations (saved recordings + transcripts) ---

#[tauri::command]
fn save_dictation(
    app: tauri::AppHandle,
    audio_base64: String,
    mime: String,
    text: String,
    answer: Option<String>,
    duration_ms: Option<u64>,
) -> Result<dictations::Dictation, String> {
    dictations::save(
        &app,
        &audio_base64,
        &mime,
        &text,
        answer,
        duration_ms,
    )
}

#[tauri::command]
fn list_dictations(app: tauri::AppHandle, query: Option<String>) -> Vec<dictations::Dictation> {
    dictations::list(&app, query.as_deref())
}

#[tauri::command]
fn delete_dictation(app: tauri::AppHandle, id: String) -> Result<(), String> {
    dictations::delete(&app, &id)
}

/// The recording as a `data:` URL for the <audio> player.
#[tauri::command]
fn get_dictation_audio(app: tauri::AppHandle, id: String) -> Result<String, String> {
    dictations::audio_data_url(&app, &id)
}

// --- Clipboard history ---

#[tauri::command]
fn get_clipboard_history(app: tauri::AppHandle) -> Vec<clipboard_hist::ClipEntry> {
    clipboard_hist::get_history(&app)
}

#[tauri::command]
fn copy_to_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    clipboard_hist::copy_to_clipboard(&app, &text)
}

// --- Calendar ---

#[tauri::command]
fn add_calendar_event(
    app: tauri::AppHandle,
    title: String,
    start: String,
    duration_minutes: Option<i64>,
) -> Result<calendar::CalendarEvent, String> {
    let _ = &app; // calendar talks to the OS directly
    calendar::add_event(&title, &start, duration_minutes.unwrap_or(30))
}

#[tauri::command]
fn list_calendar_events(
    app: tauri::AppHandle,
    scope: String,
) -> Result<Vec<calendar::CalendarEvent>, String> {
    let _ = &app;
    calendar::list_events(&scope)
}

// --- AI / agent ---

/// Multi-turn agent chat with streamed thinking/output. Events flow through
/// `channel`; the final answer also arrives as `Done`.
/// NOTE: must be `async` — sync commands run on the main thread and would
/// block event delivery, so the streaming work is moved to spawn_blocking.
#[tauri::command]
async fn chat_stream(
    app: tauri::AppHandle,
    history: Vec<ai::ChatMsg>,
    message: String,
    channel: tauri::ipc::Channel<ai::StreamEvent>,
) -> Result<(), String> {
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        ai::run_agent_stream(
            &app,
            &settings.openai_api_key,
            &settings.chat_base_url,
            &settings.chat_model,
            &history,
            &message,
            &channel,
        )
    })
    .await
    .map_err(|e| format!("agent task failed: {e}"))?
}

/// One-shot agent turn (used by the dictate window).
#[tauri::command]
async fn run_agent(app: tauri::AppHandle, message: String) -> Result<String, String> {
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        ai::run_agent(
            &app,
            &settings.openai_api_key,
            &settings.chat_base_url,
            &settings.chat_model,
            &message,
        )
    })
    .await
    .map_err(|e| format!("agent task failed: {e}"))?
}

#[tauri::command]
async fn transcribe_audio(
    app: tauri::AppHandle,
    audio_base64: String,
    mime: String,
) -> Result<String, String> {
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        ai::transcribe(
            &settings.openai_api_key,
            &settings.chat_base_url,
            &audio_base64,
            &mime,
        )
    })
    .await
    .map_err(|e| format!("transcription task failed: {e}"))?
}

/// Copy dictated text and notify the user (paste with Cmd/Ctrl+V anywhere).
#[tauri::command]
fn dictate_to_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    clipboard_hist::copy_to_clipboard(&app, &text)?;
    app.notification()
        .builder()
        .title("Supacast dictate")
        .body("Transcript copied — paste with Cmd/Ctrl+V")
        .show()
        .map_err(|e| e.to_string())
}

/// Simulate a Cmd/Ctrl+V keystroke so the paste lands in whatever input
/// had focus in the previously-used app.
fn paste_keystroke() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta // Command on macOS
    } else {
        Key::Control
    };
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Dictate (Text): copy to clipboard, return focus to the previous app and
/// paste at its cursor. Falls back to a notification if the keystroke
/// injection is blocked (e.g. missing Accessibility permission on macOS).
#[tauri::command]
async fn paste_to_focused_app(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    clipboard_hist::copy_to_clipboard(&app, &text)?;
    // The ring stays visible in its "done" state (with the transcript) so
    // the user can keep dictating — the global Enter capture stays active
    // until the ring is closed (✕ / Esc / opening the launcher).
    // Bring the original paste target back to the front so the keystroke
    // lands in the user's text field. AppKit wants the main thread.
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let app_for_restore = app.clone();
        let _ = app_for_restore.run_on_main_thread(move || {
            let _ = tx.send(paste_focus::restore());
        });
        let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
    }
    // Give the OS a beat to restore focus to the previously-used app.
    std::thread::sleep(std::time::Duration::from_millis(300));
    // The synthesized keystroke must run on the main thread: enigo's macOS
    // implementation queries HIToolbox text-input APIs that are dispatch-
    // asserted to the main queue (SIGTRAP — crash — otherwise).
    #[cfg(target_os = "macos")]
    {
        let app_for_paste = app.clone();
        let _ = app_for_paste.clone().run_on_main_thread(move || {
            if paste_keystroke().is_err() {
                let _ = app_for_paste
                    .notification()
                    .builder()
                    .title("Supacast dictate")
                    .body("Copied to clipboard — paste with Cmd/Ctrl+V")
                    .show();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    if paste_keystroke().is_err() {
        let _ = app
            .notification()
            .builder()
            .title("Supacast dictate")
            .body("Copied to clipboard — paste with Cmd/Ctrl+V")
            .show();
    }
    Ok(())
}

/// Dictate (Supacast): hand the finished Q&A to the launcher. Hides the
/// dictate window, shows the launcher (without the launcher-shown reset)
/// and emits the answer so the chat view displays it.
#[tauri::command]
fn dictate_show_answer(
    app: tauri::AppHandle,
    message: String,
    answer: String,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("dictate") {
        let _ = window.hide();
    }
    if let Some(window) = app.get_webview_window("main") {
        position_launcher(&window);
        let _ = window.show();
        let _ = window.set_focus();
    }
    // Dictation finished — the launcher takes over from here.
    set_dictate_capture(&app, false);
    app.emit(
        "dictate-answer",
        serde_json::json!({ "message": message, "answer": answer }),
    )
    .map_err(|e| e.to_string())
}

// --- Dictate window ---

/// Latest desired state of the global Enter capture (true = ring visible).
static DICTATE_CAPTURE_DESIRED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Serialises capture state changes so a stale worker can't override a
/// newer request.
static DICTATE_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// While the dictate ring is visible, a global OS-wide "Enter" hotkey is
/// registered and its press/release events are forwarded to the ring as
/// `dictate-key` events. This makes hold-Enter-to-record work even when the
/// ring window does not have keyboard focus (e.g. while typing in another
/// app). The hotkey is removed as soon as dictation ends so regular apps
/// never lose their Enter key.
///
/// The plugin's register/unregister dispatch to the main thread and block
/// the caller until it runs. This must never happen on the main thread
/// itself (hotkey handler, sync command) or the event loop deadlocks — so
/// the state change is always applied from a worker thread.
fn set_dictate_capture(app: &tauri::AppHandle, enabled: bool) {
    use std::sync::atomic::Ordering;

    DICTATE_CAPTURE_DESIRED.store(enabled, Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        let _guard = DICTATE_CAPTURE_LOCK.lock().unwrap();
        // Re-read inside the lock so the most recent request wins even if
        // worker threads run out of order.
        let enabled = DICTATE_CAPTURE_DESIRED.load(Ordering::SeqCst);

        let sc: Shortcut = match "Enter".parse() {
            Ok(sc) => sc,
            Err(_) => {
                eprintln!("could not parse Enter shortcut");
                return;
            }
        };
        if enabled {
            if app.global_shortcut().is_registered(sc) {
                return;
            }
            if let Err(e) = app.global_shortcut().on_shortcut(sc, |app, _sc, event| {
                let payload = if event.state == ShortcutState::Pressed {
                    // The app that's frontmost when Enter goes down is the
                    // paste target (the user may have switched apps while
                    // the ring floats on top).
                    #[cfg(target_os = "macos")]
                    paste_focus::capture();
                    "down"
                } else {
                    "up"
                };
                let _ = app.emit("dictate-key", payload.to_string());
            }) {
                eprintln!("could not capture global Enter: {e}");
            }
        } else if app.global_shortcut().is_registered(sc) {
            let _ = app.global_shortcut().unregister(sc);
        }
    });
}

#[tauri::command]
fn open_dictate(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    // Remember the user's paste target before the ring takes focus.
    #[cfg(target_os = "macos")]
    paste_focus::capture();
    if let Some(window) = app.get_webview_window("dictate") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("dictate-mode", mode);
    }
    set_dictate_capture(&app, true);
    Ok(())
}

/// Hide the dictate ring and release the global Enter capture.
#[tauri::command]
fn close_dictate(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("dictate") {
        let _ = window.hide();
    }
    set_dictate_capture(&app, false);
}

// ---------------------------------------------------------------------------
// Shortcut normalisation
// ---------------------------------------------------------------------------

/// Clip a message for notification display (notifications don't wrap well).
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let clipped: String = s.chars().take(max).collect();
        format!("{clipped}…")
    }
}

fn normalize_shortcut(raw: &str) -> String {
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
            "cmd" | "command" | "super" | "meta" => "Cmd",
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

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

/// Check GitHub Releases for a newer signed bundle and, if found, download
/// and install it, then restart the app. Uses the updater plugin configured
/// in tauri.conf.json (`plugins.updater` — endpoint serves `latest.json`
/// from the latest GitHub release, produced by .github/workflows/release.yml).
fn check_for_updates(app: &tauri::AppHandle) {
    use tauri_plugin_notification::NotificationExt;
    use tauri_plugin_updater::UpdaterExt;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let notify = |title: &str, body: &str| {
            let _ = app
                .notification()
                .builder()
                .title(title)
                .body(body)
                .show();
        };

        let result: Result<bool, String> = async {
            let updater = app.updater().map_err(|e| e.to_string())?;
            // None means the current version is the newest known release.
            let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
                return Ok(false);
            };
            update
                .download_and_install(|_chunk, _total| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            Ok(true)
        }
        .await;

        match result {
            Ok(false) => notify("Supacast", "You're on the latest version."),
            Ok(true) => {
                // Give the notification a beat before the process is replaced.
                notify("Supacast", "Update downloaded — restarting to install…");
                std::thread::sleep(std::time::Duration::from_secs(1));
                app.restart();
            }
            Err(e) => notify("Supacast update failed", &truncate_str(&e, 160)),
        }
    });
}

fn show_launcher(app: &tauri::AppHandle) {
    // Opening the launcher exits dictation: release the global Enter capture.
    set_dictate_capture(app, false);
    if let Some(window) = app.get_webview_window("main") {
        position_launcher(&window);
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("launcher-shown", ());
    }
}

fn toggle_launcher(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(window) => {
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
            } else {
                show_launcher(app);
            }
        }
        None => eprintln!("main window not found"),
    }
}

fn show_settings(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("settings-shown", ());
    }
}

/// Center the launcher on the screen (Spotlight style).
fn position_launcher(window: &WebviewWindow) {
    if let Ok(Some(monitor)) = window.current_monitor() {
        if let Ok(win_size) = window.outer_size() {
            let scale = monitor.scale_factor();
            let win_w = win_size.width as f64 / scale;
            let win_h = win_size.height as f64 / scale;
            let mon_w = monitor.size().width as f64 / scale;
            let mon_h = monitor.size().height as f64 / scale;
            let x = (mon_w - win_w) / 2.0;
            let y = (mon_h - win_h) / 2.0;
            let _ = window.set_position(tauri::LogicalPosition::new(x, y));
        }
    }
}

/// Frosted-glass effect: NSVisualEffectView on macOS, acrylic on Windows.
fn apply_glass(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = window_vibrancy::apply_vibrancy(
            window,
            window_vibrancy::NSVisualEffectMaterial::HudWindow,
            Some(window_vibrancy::NSVisualEffectState::Active),
            Some(0.0), // square corners
        ) {
            eprintln!("vibrancy not applied: {e}");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Err(e) = window_vibrancy::apply_acrylic(
            window,
            Some((24, 26, 34, 140)), // dark translucent tint
        ) {
            eprintln!("acrylic not applied: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// App bootstrap
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_launcher(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None, // no extra args — Supacast starts hidden in the tray
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            search_apps,
            search_files,
            open_path,
            hide_launcher,
            open_settings,
            add_todo,
            list_todos,
            complete_todo,
            set_todo_done,
            delete_todo,
            add_note,
            list_notes,
            delete_note,
            get_clipboard_history,
            copy_to_clipboard,
            save_dictation,
            list_dictations,
            delete_dictation,
            get_dictation_audio,
            add_calendar_event,
            list_calendar_events,
            chat_stream,
            run_agent,
            transcribe_audio,
            dictate_to_text,
            paste_to_focused_app,
            dictate_show_answer,
            open_dictate,
            close_dictate,
        ])
        .setup(|app| {
            // Menu-bar-only app on macOS: no dock icon, lives in the tray.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();

            // --- Launch at login ---
            // Supacast always starts with the system after install; the
            // plugin is idempotent so re-enabling on every launch is safe.
            if let Err(e) = handle.autolaunch().enable() {
                eprintln!("could not enable launch-at-login: {e}");
            }

            // --- Tray menu ---
            let open_item = MenuItem::with_id(app, "open", "Open Supacast", true, None::<&str>)?;
            let dictate_item =
                MenuItem::with_id(app, "dictate", "Dictate (Supacast)", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let updates_item =
                MenuItem::with_id(app, "check-updates", "Check for Updates…", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Supacast", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open_item,
                    &PredefinedMenuItem::separator(app)?,
                    &dictate_item,
                    &PredefinedMenuItem::separator(app)?,
                    &settings_item,
                    &updates_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_item,
                ],
            )?;

            TrayIconBuilder::with_id("supacast-tray")
                .icon(
                    tauri::image::Image::from_bytes(include_bytes!(
                        "../icons/tray/rocket-32.png"
                    ))
                    .expect("failed to decode tray icon"),
                )
                .icon_as_template(true) // macOS: adapt to menu bar appearance
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_launcher(app),
                    "dictate" => {
                        show_launcher(app);
                        let _ = app.emit("open-dictate", "supacast".to_string());
                    }
                    "settings" => show_settings(app),
                    "check-updates" => check_for_updates(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // --- Global shortcut (from persisted settings) ---
            let settings = settings::load(handle);
            let shortcut: Shortcut = settings
                .shortcut
                .parse()
                .unwrap_or_else(|_| settings::default_shortcut().parse().unwrap());
            app.global_shortcut()
                .on_shortcut(shortcut, |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_launcher(app);
                    }
                })
                .expect("failed to register global shortcut");

            // --- Background loops ---
            todos::spawn_reminder_loop(handle.clone());
            clipboard_hist::spawn_clipboard_loop(handle.clone());

            // Frosted glass for launcher + settings windows.
            for label in ["main", "settings"] {
                if let Some(window) = app.get_webview_window(label) {
                    apply_glass(&window);
                }
            }

            // Position the (hidden) launcher window, centered.
            if let Some(window) = app.get_webview_window("main") {
                position_launcher(&window);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Spotlight behaviour: clicking away hides the launcher.
            WindowEvent::Focused(false) => {
                if window.label() == "main" {
                    let _ = window.hide();
                }
            }
            // Keep the settings/dictate windows alive; hide instead of
            // destroying so they can be reopened.
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "settings" || window.label() == "dictate" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Supacast");
}
