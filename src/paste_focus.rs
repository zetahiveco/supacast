//! Remembers which app was frontmost when dictation started and brings it
//! back to the front right before the paste keystroke fires. Without this,
//! hiding the dictate ring leaves Supacast (or whatever had focus) active
//! and the synthesized Cmd+V never reaches the user's text field.

#![cfg(target_os = "macos")]

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use std::sync::Mutex;

static PREVIOUS_PID: Mutex<Option<i32>> = Mutex::new(None);

fn workspace() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"NSWorkspace")?;
    // SAFETY: class method call with a valid receiver; result is retained.
    unsafe { Some(msg_send![cls, sharedWorkspace]) }
}

/// Remember which app is frontmost right now — this is the paste target.
/// Skips Supacast itself so we never try to "restore" into our own app.
pub fn capture() {
    let Some(ws) = workspace() else { return };
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![&ws, frontmostApplication];
        if let Some(app) = app {
            let pid: i32 = msg_send![&app, processIdentifier];
            if pid > 0 && pid != std::process::id() as i32 {
                *PREVIOUS_PID.lock().unwrap() = Some(pid);
            }
        }
    }
}

/// Bring the remembered app back to the front. Must be called on the main
/// thread (AppKit requirement). Returns true when activation was accepted.
pub fn restore() -> bool {
    let Some(pid) = *PREVIOUS_PID.lock().unwrap() else {
        return false;
    };
    if pid <= 0 || pid == std::process::id() as i32 {
        return false;
    }
    let Some(cls) = AnyClass::get(c"NSRunningApplication") else {
        return false;
    };
    unsafe {
        let app: Option<Retained<AnyObject>> =
            msg_send![cls, runningApplicationWithProcessIdentifier: pid];
        let Some(app) = app else { return false };
        // NSApplicationActivationAllWindows (1<<0) | IgnoringOtherApps (1<<1)
        let options: usize = (1 << 0) | (1 << 1);
        let ok: bool = msg_send![&app, activateWithOptions: options];
        ok
    }
}