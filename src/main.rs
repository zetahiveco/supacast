//! Supacast (pure Rust) — a fast, keyboard-first launcher for apps, files
//! & AI. Native egui UI replacing the Tauri webview version; all backend
//! logic is the same Rust code with the Tauri dependencies removed and
//! every web API swapped for a native one:
//!
//! | Web (Tauri)                    | Native (this crate)              |
//! |--------------------------------|----------------------------------|
//! | navigator.mediaDevices + MediaRecorder | cpal input stream + WAV |
//! | AudioContext analyser (level)  | RMS over cpal callback           |
//! | <audio> player                 | cpal output stream               |
//! | tauri-plugin-clipboard-manager | arboard                          |
//! | tauri-plugin-global-shortcut   | global-hotkey                    |
//! | tauri-plugin-notification      | notify-rust                      |
//! | tauri-plugin-autostart         | auto-launch                      |
//! | tauri-plugin-single-instance   | single-instance                  |
//! | tauri-plugin-updater           | GitHub latest.json check         |
//! | Tauri tray + menu              | tray-icon                        |
//! | tauri IPC Channel (streaming)  | std::sync::mpsc                  |
//! | Tauri windows (3 webviews)     | one morphing egui root window    |

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod app;
mod audio;
mod calendar;
mod clipboard_hist;
mod dictations;
mod events;
mod fonts;
mod hotkeys;
mod notes;
mod paste_focus;
mod paths;
mod platform;
mod search;
mod settings;
mod todos;
mod ui;

use std::sync::{Arc, Mutex, OnceLock};

use app::App;
use events::UiEvent;

fn main() {
    // --- Single instance ---
    // The single-instance crate treats the name as a LITERAL file path (on
    // macOS it flocks it), so it must be absolute and in a always-writable
    // location. A relative name resolves against the process CWD — which is
    // `/` (read-only) for `open`/Finder/launchd launches, crashing the app
    // with "Read-only file system" before anything else runs.
    let lock_path = paths::data_dir().map(|dir| {
        std::fs::create_dir_all(&dir).ok();
        dir.join("instance.lock")
    });
    let instance = lock_path.and_then(|path| {
        single_instance::SingleInstance::new(&path.to_string_lossy()).ok()
    });
    match instance {
        Some(inst) if !inst.is_single() => {
            eprintln!("Supacast is already running.");
            return;
        }
        Some(inst) => {
            // Hold the lock for the process lifetime (dropping it would
            // release the flock and let a second instance start).
            std::mem::forget(inst);
        }
        None => {
            // Degrade gracefully: a stale/failed guard must never crash the
            // app — worst case two instances run briefly.
            eprintln!("could not create single-instance guard; continuing without it");
        }
    }

    // --- Event plumbing ---
    let (tx, rx) = std::sync::mpsc::channel::<UiEvent>();

    let settings = settings::load();
    // The hotkey manager is created lazily on the first main-thread
    // `logic()` call — macOS needs the event loop running first (see
    // Shared::ensure_hotkeys).
    let shared = Arc::new(app::Shared {
        ctx: OnceLock::new(),
        events_tx: tx.clone(),
        hotkeys: Mutex::new(None),
        settings: Mutex::new(settings.clone()),
        dictate: Mutex::new(events::DictateState::default()),
        dictate_history: Mutex::new(Vec::new()),
        recorder: Mutex::new(None),
        enter_held: std::sync::atomic::AtomicBool::new(false),
    });

    // --- Background loops ---
    todos::spawn_reminder_loop(shared.clone());
    clipboard_hist::spawn_clipboard_loop();

    // Microphone check at launch: if the default input yields no audio
    // (permission not granted yet, or denied — a denied permission captures
    // a silent stream), fire the TCC prompt on macOS and notify. Without
    // this, dictation just reports "Nothing heard".
    std::thread::spawn(|| {
        if !audio::probe_input(3000) {
            #[cfg(target_os = "macos")]
            platform::macos::request_mic_permission();
            platform::notify(
                "Supacast — no microphone input",
                "Allow microphone access for Supacast (System Settings → Privacy & Security → Microphone) and relaunch.",
            );
        }
    });

    // Supacast always starts with the system after install; the plugin is
    // idempotent so re-enabling on every launch is safe.
    platform::enable_autostart();

    // Startup hello: confirms the reminder/notify pipeline is alive each
    // time the app launches (also pre-builds the notifier applet).
    platform::notify(
        "Supacast is running",
        "You will receive notifications and remainders from the app",
    );

    // Menu-bar-only app on macOS: no dock icon, lives in the tray.
    #[cfg(target_os = "macos")]
    set_accessory_policy();

    // Tray menu events are polled on the main thread in `App::pump_events`.
    let tray = platform::create_tray().expect("failed to create tray icon");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Supacast")
            .with_inner_size([app::LAUNCHER_SIZE.0, app::LAUNCHER_SIZE.1])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_visible(false), // starts hidden in the tray
        ..Default::default()
    };

    eframe::run_native(
        "Supacast",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, shared, rx, tray)) as Box<dyn eframe::App>)),
    )
    .expect("error while running Supacast");
}

/// Switch the macOS app to an accessory (menu-bar-only) application.
#[cfg(target_os = "macos")]
fn set_accessory_policy() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        // NSApplicationActivationPolicy::Accessory == 1; the selector
        // returns BOOL, so declare the return type as `bool` to keep
        // objc2's message-send type check happy.
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return;
        }
        let _: bool = msg_send![app, setActivationPolicy: 1i64];
    }
}

/// Bring the app to the foreground so its windows can take keyboard focus.
/// Necessary because we're an accessory (menu-bar-only) app: without an
/// explicit activation, shown windows never become key and egui reports
/// them as unfocused. (Tauri did this for us via window `focus: true`.)
#[cfg(target_os = "macos")]
pub fn activate_app() {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return;
        }
        // Deprecated on macOS 14+ but still the most reliable way to take
        // focus from a background/accessory app.
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app() {}
