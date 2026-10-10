//! OS integration helpers — replace the Tauri notification / autostart /
//! updater plugins and the tray builder.

use crate::events::UiEvent;

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Rocket icon (violetish-silver rocket on a black-gray rounded square,
/// padded) baked into the notification applet below.
#[cfg(target_os = "macos")]
const NOTIFICATION_ICON_PNG: &[u8] = include_bytes!("../assets/rocket-icon-512.png");

/// Swift UNUserNotificationCenter helper (compiled from assets/notifier.swift
/// with `swiftc -O`), installed as the app bundle's executable.
#[cfg(target_os = "macos")]
const NOTIFIER_BIN: &[u8] = include_bytes!("../assets/supacast-notify");

/// Version stamp for the applet's icon — bump to force a reinstall.
#[cfg(target_os = "macos")]
const NOTIFIER_ICON_VERSION: &str = "5";

/// Info.plist for the notification applet bundle. UNUserNotificationCenter
/// requires a bundle id, and macOS 15+ requires NSUserNotificationsUsage
/// Description or notification APIs silently no-op. The display name is
/// "Supacast" so the Notification Center stack isn't labelled
/// "SupacastNotifier".
#[cfg(target_os = "macos")]
const NOTIFIER_INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>applet</string>
    <key>CFBundleIdentifier</key><string>com.supacast.notify2</string>
    <key>CFBundleName</key><string>Supacast</string>
    <key>CFBundleDisplayName</key><string>Supacast</string>
    <key>CFBundleIconFile</key><string>applet</string>
    <key>CFBundleIconName</key><string>applet</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleVersion</key><string>1.0.0</string>
    <key>CFBundleShortVersionString</key><string>1.0.0</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSUserNotificationsUsageDescription</key><string>Supacast posts reminders for your todos.</string>
</dict>
</plist>
"#;

/// Build (once) a tiny app bundle in the app data dir and post
/// notifications through it, so banners carry Supacast's rocket icon
/// instead of Script Editor's (the host of a plain `osascript` run).
#[cfg(target_os = "macos")]
fn notifier_applet() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    static ONCE: std::sync::Once = std::sync::Once::new();
    static EXE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ONCE.call_once(|| {
        EXE.set(build_applet()).ok();
    });
    EXE.get().cloned().flatten()
}

/// Compile the notifier applet (skipping work already done) and return the
/// path of its executable, or None when it can't be built.
#[cfg(target_os = "macos")]
fn build_applet() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let dir = crate::paths::data_dir()?;
    let app = dir.join("SupacastNotifier.app");
    let exe = app.join("Contents/MacOS/applet");
    let icns = app.join("Contents/Resources/applet.icns");
    let stamp = dir.join(".notifier-icon-ver");
    let stamped = |p: &std::path::Path| {
        std::fs::read_to_string(p).is_ok_and(|s| s.trim() == NOTIFIER_ICON_VERSION)
    };

    // --- Bundle skeleton + executable (rebuilt when missing) ---
    if !exe.exists() {
        let macos_dir = app.join("Contents/MacOS");
        let res_dir = app.join("Contents/Resources");
        std::fs::create_dir_all(&macos_dir).ok()?;
        std::fs::create_dir_all(&res_dir).ok()?;
        std::fs::write(app.join("Contents/Info.plist"), NOTIFIER_INFO_PLIST).ok()?;
        std::fs::write(app.join("Contents/PkgInfo"), "APPL????").ok()?;
        std::fs::write(&exe, NOTIFIER_BIN).ok()?;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).ok()?;
    }

    // --- Icon: install the rocket icns when the stamp is stale ---
    if stamped(&stamp) && icns.exists() {
        return Some(exe);
    }
    let png = dir.join("rocket-icon-512.png");
    std::fs::write(&png, NOTIFICATION_ICON_PNG).ok()?;
    let ok = Command::new("sips")
        .args(["-s", "format", "icns"])
        .arg(&png)
        .arg("--out")
        .arg(&icns)
        .output()
        .is_ok_and(|o| o.status.success());
    let _ = std::fs::remove_file(&png);
    if !ok {
        eprintln!("could not convert rocket icon to icns");
        return None;
    }
    std::fs::write(&stamp, NOTIFIER_ICON_VERSION).ok();
    // Changing the executable/resources invalidates the ad-hoc signature,
    // so re-sign — then re-register with Launch Services so the system
    // picks up the new icon/name (banners cache both per bundle).
    let _ = Command::new("codesign").args(["--force", "-s", "-"]).arg(&app).output();
    let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
        .args(["-f", "-R"])
        .arg(&app)
        .output();
    Some(exe)
}

/// Path of the file the Swift helper touches once notification permission
/// is granted (lets the caller skip the permission-prompt bootstrap).
#[cfg(target_os = "macos")]
fn notifier_auth_stamp() -> Option<std::path::PathBuf> {
    crate::paths::data_dir().map(|d| d.join(".notifier-auth-ok"))
}

/// Fire a desktop notification (best-effort).
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let title = title.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let Some(exe) = notifier_applet() else {
                eprintln!("notifier applet unavailable; falling back to osascript");
                osascript_notify(&title, &body);
                return;
            };
            let app = exe
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf());

            // Direct post — works once notification permission is granted.
            let direct = |exe: &std::path::Path| {
                let mut cmd = std::process::Command::new(exe);
                cmd.arg(&title).arg(&body);
                if let Some(stamp) = notifier_auth_stamp() {
                    cmd.env("SUPACAST_AUTH_STAMP", stamp);
                }
                cmd.output().is_ok_and(|o| o.status.success())
            };
            let mut ok = direct(&exe);

            // Authorization can be lost when the bundle is rebuilt (ad-hoc
            // identity changes) — and a directly-spawned process can't show
            // the permission prompt (macOS denies silently). Launch the
            // applet through launchd so the prompt can appear, then retry
            // the direct post. Attempted once per process.
            static BOOTSTRAPPED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !ok
                && !BOOTSTRAPPED.swap(
                    true,
                    std::sync::atomic::Ordering::Relaxed,
                )
            {
                eprintln!("notifier not authorized; bootstrapping via open");
                if let Some(app) = &app {
                    // Foreground (no -g): macOS only presents the
                    // notification permission prompt to an active app.
                    let _ = std::process::Command::new("open")
                        .arg(app)
                        .args(["--args", &title, &body])
                        .status();
                }
                ok = direct(&exe);
            }

            if ok {
                return;
            }
            eprintln!("notifier applet failed; falling back to osascript");
            osascript_notify(&title, &body);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        // notify-rust wants to be driven from a thread with a run loop;
        // notifications are best-effort anyway.
        let title = title.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let _ = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .show();
        });
    }
}

/// Fallback: bare osascript — no custom icon (attributed to Script
/// Editor), but reliably delivered.
#[cfg(target_os = "macos")]
fn osascript_notify(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\" sound name \"Glass\"",
        applescript_escape(body),
        applescript_escape(title),
    );
    let script = script.replace('\n', " "); // AppleScript strings are single-line
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output();
}

/// Escape a string for interpolation inside a double-quoted AppleScript
/// literal: backslashes and double quotes.
#[cfg(target_os = "macos")]
fn applescript_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
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
    use objc2::AnyThread;
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSImage, NSView,
        NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
        NSVisualEffectView, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSData, NSString};
    use std::sync::Mutex;

    /// Set the Dock icon for the running app. Supacast is a bare binary,
    /// so without this the Dock shows the generic executable icon while
    /// the app runs. Call once on the main thread after AppKit is up.
    pub fn set_dock_icon(png: &[u8]) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("not on main thread")?;
        let data = NSData::from_vec(png.to_vec());
        let img = NSImage::initWithData(NSImage::alloc(), &data)
            .ok_or("invalid icon png")?;
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img));
        }
        Ok(())
    }

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
