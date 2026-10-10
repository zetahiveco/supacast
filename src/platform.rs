//! OS integration helpers — replace the Tauri notification / autostart /
//! updater plugins and the tray builder.

use crate::events::UiEvent;

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Fire a desktop notification (best-effort).
pub fn notify(title: &str, body: &str) {
    // notify-rust wants to be driven from a thread with a run loop on macOS;
    // show() from a worker thread is fine in practice, and notifications are
    // best-effort anyway.
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .summary(&title)
            .body(&body)
            .show();
    });
}

// ---------------------------------------------------------------------------
// Launch at login
// ---------------------------------------------------------------------------

/// Enable launch-at-login. Idempotent; re-enabled on every launch.
pub fn enable_autostart() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not resolve current exe for autostart: {e}");
            return;
        }
    };
    let launch = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("Supacast")
        .set_app_path(&exe.to_string_lossy())
        .build();
    match launch {
        Ok(l) => {
            if let Err(e) = l.enable() {
                eprintln!("could not enable launch-at-login: {e}");
            }
        }
        Err(e) => eprintln!("could not build autostart entry: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Software updates
// ---------------------------------------------------------------------------

const RELEASES_URL: &str = "https://github.com/zetahiveco/supacast/releases/latest";
const LATEST_JSON: &str =
    "https://github.com/zetahiveco/supacast/releases/latest/download/latest.json";

/// Check GitHub Releases for a newer version and notify the user (opens the
/// releases page — no in-place binary swap like the Tauri updater, but no
/// signing infrastructure required either).
pub fn check_for_updates(shared: std::sync::Arc<crate::app::Shared>) {
    let notify_tx = shared.events_tx.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<bool, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())?;
            let v: serde_json::Value = client
                .get(LATEST_JSON)
                .send()
                .map_err(|e| format!("request failed: {e}"))?
                .json()
                .map_err(|e| format!("bad response: {e}"))?;
            let latest = v["version"].as_str().unwrap_or("").trim_start_matches('v');
            let current = env!("CARGO_PKG_VERSION");
            if !latest.is_empty() && latest != current && version_gt(latest, current) {
                open_url(RELEASES_URL);
                Ok(true)
            } else {
                Ok(false)
            }
        })();
        match result {
            Ok(true) => notify_tx
                .send(UiEvent::Notify {
                    title: "Supacast update".into(),
                    body: "A new version was found — opening the download page.".into(),
                })
                .ok(),
            Ok(false) => notify_tx
                .send(UiEvent::Notify {
                    title: "Supacast".into(),
                    body: "You're on the latest version.".into(),
                })
                .ok(),
            Err(e) => notify_tx
                .send(UiEvent::Notify {
                    title: "Supacast update failed".into(),
                    body: e,
                })
                .ok(),
        };
        shared.repaint(); // wake the UI loop so the event is processed
    });
}

fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(a) > parse(b)
}

/// Open a URL with the OS default handler.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

// ---------------------------------------------------------------------------
// macOS: frosted-glass window vibrancy + microphone permission
// ---------------------------------------------------------------------------

/// AppKit/AVFoundation shims via objc2. The vibrancy view is the same thing
/// `window_vibrancy::apply_vibrancy` did for the Tauri app: an
/// NSVisualEffectView (HUD material, behind-window blending) under the
/// window's content view — real backdrop blur, unlike a translucent fill.
#[cfg(target_os = "macos")]
pub mod macos {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
        NSWindowOrderingMode,
    };
    use objc2_foundation::NSString;
    use std::sync::Mutex;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    /// Strongly-retained pointer to the installed NSVisualEffectView
    /// (+1 retain this module owns). Checked/dereferenced only via
    /// `Retained::from_raw` so a view AppKit has removed and deallocated
    /// is detected safely instead of trapping on a dangling pointer.
    static EFFECT_VIEW: Mutex<Option<usize>> = Mutex::new(None);

    /// Whether the frosted layer should currently be hidden (dictate ring
    /// hides it so only its own glass disc is visible). Remembered so a
    /// re-installed blur view comes back in the right state.
    static VIBRANCY_HIDDEN: Mutex<bool> = Mutex::new(false);

    /// Install the frosted-glass layer for the app's "Supacast" window:
    ///
    /// ```text
    /// theme frame (content.superview)
    ///   ├─ NSVisualEffectView (HUD material, behind-window blending)  ← bottom
    ///   └─ the winit/GL view (the app's UI, still the contentView)    ← top
    /// ```
    ///
    /// The blur must live *behind* the contentView, not inside it: winit
    /// (0.30) casts `contentView()` back to its own view class on every
    /// cursor/IME call, so the GL view may neither be replaced nor
    /// re-parented. And a blur view added *inside* the contentView renders
    /// above the GL framebuffer (sublayers composite over the parent
    /// layer's own content), which blanks the UI. As a sibling behind the
    /// contentView it shows through the transparent framebuffer.
    ///
    /// Idempotent — safe to call every tick. Toggling decorations (the
    /// settings window uses the native title bar) can rebuild the theme
    /// frame and drop the blur, so the caller re-invokes this to restore it.
    pub fn apply_vibrancy() -> Result<(), String> {
        // The app is running on the main thread (eframe event loop); this is
        // only called from `App::logic`.
        let mtm = MainThreadMarker::new().ok_or("not on main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .windows()
            .iter()
            // Settings mode retitles the window ("Supacast Settings").
            .find(|w| w.title().hasPrefix(&NSString::from_str("Supacast")))
            .ok_or("Supacast window not found")?;
        let content: Retained<NSView> = window
            .contentView()
            .ok_or("window has no content view")?;
        let parent = unsafe { content.superview() }
            .ok_or("content view has no parent (theme frame)")?;

        // Still installed? Take our strong reference out while checking so
        // a removed-and-released view is detected without touching freed
        // memory (the previous weak-pointer check crashed with SIGTRAP).
        let existing = EFFECT_VIEW.lock().unwrap().take();
        if let Some(ptr) = existing {
            if let Some(view) = unsafe { Retained::<NSView>::from_raw(ptr as *mut NSView) } {
                if view.window().is_some() {
                    // Alive and in a window — put the strong ref back.
                    *EFFECT_VIEW.lock().unwrap() =
                        Some(Retained::into_raw(view) as usize);
                    return Ok(());
                }
                // Removed from the hierarchy; dropping `view` here releases
                // our retain (and deallocates it if nothing else held it).
            }
        }

        let bounds = parent.bounds();
        let blur = NSVisualEffectView::new(mtm);
        blur.setMaterial(NSVisualEffectMaterial::HUDWindow);
        blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        blur.setState(NSVisualEffectState::Active);
        blur.setFrame(bounds);
        blur.setAutoresizingMask(NSAutoresizingMaskOptions(2 | 16));
        blur.setHidden(*VIBRANCY_HIDDEN.lock().unwrap());
        parent.addSubview_positioned_relativeTo(&blur, NSWindowOrderingMode::Below, Some(&content));

        *EFFECT_VIEW.lock().unwrap() = Some(Retained::into_raw(blur) as usize);
        Ok(())
    }

    /// Show/hide the frosted-glass layer. The dictate ring hides it so only
    /// its own glass disc is visible (matching the React window, which was
    /// fully transparent apart from the disc). Remembered across re-installs.
    pub fn set_vibrancy_hidden(hidden: bool) {
        *VIBRANCY_HIDDEN.lock().unwrap() = hidden;
        let existing = EFFECT_VIEW.lock().unwrap().take();
        if let Some(ptr) = existing {
            if let Some(view) = unsafe { Retained::<NSView>::from_raw(ptr as *mut NSView) } {
                view.setHidden(hidden);
                // Put the strong ref back (the view is still installed).
                *EFFECT_VIEW.lock().unwrap() = Some(Retained::into_raw(view) as usize);
            }
        }
    }

    /// Current macOS mic-permission status: `Some(true)` granted,
    /// `Some(false)` denied, `None` not determined yet.
    pub fn mic_permission_status() -> Option<bool> {
        let device_cls = AnyClass::get(c"AVCaptureDevice")?;
        unsafe {
            let media_type = &*NSString::from_str("soun");
            let status: i64 = msg_send![device_cls, authorizationStatusForMediaType: media_type];
            match status {
                3 => Some(true),
                1 | 2 => Some(false),
                _ => None,
            }
        }
    }

    /// Ask macOS for microphone access (shows the TCC prompt the first
    /// time). Without permission CoreAudio captures pure silence, so every
    /// dictation would transcribe to "Nothing heard".
    pub fn request_mic_permission() {
        let Some(device_cls) = AnyClass::get(c"AVCaptureDevice") else {
            eprintln!("Supacast: AVCaptureDevice not available");
            return;
        };
        unsafe {
            // AVMediaTypeAudio is the four-char code "soun".
            let media_type = &*NSString::from_str("soun");
            // AVAuthorizationStatus: 0 notDetermined, 1 restricted,
            // 2 denied, 3 authorized.
            let status: i64 = msg_send![device_cls, authorizationStatusForMediaType: media_type];
            eprintln!("Supacast: microphone permission status = {status}");
            if status == 0 {
                // Not determined yet — show the system prompt. The handler
                // is leaked: it is invoked once, asynchronously.
                let handler = RcBlock::new(move |granted: objc2::runtime::Bool| {
                    eprintln!("Supacast: microphone permission granted = {}", bool::from(granted));
                });
                let _: () = msg_send![
                    device_cls,
                    requestAccessForMediaType: media_type,
                    completionHandler: &*handler,
                ];
                std::mem::forget(handler);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tray icon
// ---------------------------------------------------------------------------

/// Build the rocket tray icon + menu (Open / Dictate / Settings / Updates /
/// Quit). Menu activations are polled on the main thread via
/// `MenuEvent::receiver()` in `App::pump_events`.
pub fn create_tray() -> Result<tray_icon::TrayIcon, String> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::TrayIconBuilder;

    // Decode the 32px rocket PNG into RGBA for the tray icon.
    let icon = load_tray_icon();

    let open_item = MenuItem::with_id("open", "Open Supacast", true, None);
    let dictate_item = MenuItem::with_id("dictate", "Dictate (Supacast)", true, None);
    let settings_item = MenuItem::with_id("settings", "Settings", true, None);
    let updates_item = MenuItem::with_id("check-updates", "Check for Updates…", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit Supacast", true, None);

    let menu = Menu::new();
    menu.append(&open_item).map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&dictate_item).map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&settings_item).map_err(|e| e.to_string())?;
    menu.append(&updates_item).map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&quit_item).map_err(|e| e.to_string())?;

    let mut builder = TrayIconBuilder::new()
        .with_id("supacast-tray")
        .with_tooltip("Supacast")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true);
    if let Some(icon) = icon {
        // On macOS draw the icon as a template image so it adapts to the
        // menu bar appearance (same as iconAsTemplate in the Tauri build).
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_icon_templated(icon);
        }
        #[cfg(not(target_os = "macos"))]
        {
            builder = builder.with_icon(icon);
        }
    }
    let tray = builder.build().map_err(|e| e.to_string())?;

    Ok(tray)
}

fn load_tray_icon() -> Option<tray_icon::Icon> {
    // The icon is embedded at compile time (assets/rocket-32.png, 32x32)
    // and decoded to RGBA with the `image` crate (png only).
    let png = include_bytes!("../assets/rocket-32.png");
    let reader = image::ImageReader::with_format(
        std::io::Cursor::new(png),
        image::ImageFormat::Png,
    );
    let img = reader.decode().ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}
