//! Desktop notifications with Supacast's rocket icon.
//!
//! Ported from the egui-era build: macOS notifications are posted through a
//! tiny app bundle (SupacastNotifier.app) built once into the app data dir,
//! so banners carry the violet rocket icon instead of the host process's
//! (a dev binary shows a generic icon — the bundled release app would show
//! the app icon, but the applet keeps the icon identical everywhere).
//! Other platforms fall back to the Tauri notification plugin.

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
/// instead of the generic/dev-process icon.
#[cfg(target_os = "macos")]
fn notifier_applet(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    static ONCE: std::sync::Once = std::sync::Once::new();
    static EXE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ONCE.call_once(|| {
        EXE.set(build_applet(data_dir)).ok();
    });
    EXE.get().cloned().flatten()
}

/// Compile the notifier applet (skipping work already done) and return the
/// path of its executable, or None when it can't be built.
#[cfg(target_os = "macos")]
fn build_applet(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let app = data_dir.join("SupacastNotifier.app");
    let exe = app.join("Contents/MacOS/applet");
    let icns = app.join("Contents/Resources/applet.icns");
    let stamp = data_dir.join(".notifier-icon-ver");
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
    let png = data_dir.join("rocket-icon-512.png");
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
    //
    // Prefer the stable self-signed "Supacast Codesign" identity (created
    // once per machine) so TCC permissions for the applet survive app
    // updates; ad-hoc fingerprints change with every rebuild and would
    // invalidate the notification grant each release.
    let signed = Command::new("codesign")
        .args(["--force", "-s", "Supacast Codesign"])
        .arg(&app)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !signed {
        let _ = Command::new("codesign").args(["--force", "-s", "-"]).arg(&app).output();
    }
    let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
        .args(["-f", "-R"])
        .arg(&app)
        .output();
    Some(exe)
}

/// Path of the file the Swift helper touches once notification permission
/// is granted (lets the caller skip the permission-prompt bootstrap).
#[cfg(target_os = "macos")]
fn notifier_auth_stamp(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    Some(data_dir.join(".notifier-auth-ok"))
}

/// Fire a desktop notification (best-effort). Safe to call from any thread;
/// the work happens on a worker so a slow applet build never blocks the
/// caller.
pub fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();

    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| eprintln!("could not resolve app data dir: {e}"))
            .ok();
        let Some(dir) = dir else { return };
        std::thread::spawn(move || {
            let Some(exe) = notifier_applet(&dir) else {
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
                if let Some(stamp) = notifier_auth_stamp(&dir) {
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
            if !ok && !BOOTSTRAPPED.swap(true, std::sync::atomic::Ordering::Relaxed) {
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
        use tauri_plugin_notification::NotificationExt;
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = app
                .notification()
                .builder()
                .title(&title)
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