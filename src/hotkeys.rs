//! Global hotkey management — replaces `tauri-plugin-global-shortcut`.
//!
//! IMPORTANT: on macOS the `GlobalHotKeyManager` must be created on the
//! **main thread** (Carbon `RegisterEventHotKey` delivers through the
//! application event target, which requires the main-thread event loop).
//! So this is a plain struct owned by `Shared` and driven from
//! `App::pump_events` — no worker thread.
//!
//! The manager registers:
//! - the launcher toggle shortcut (e.g. "CmdOrCtrl+Shift+Y"), and
//! - a global Enter capture while the dictate ring is visible
//!   (press/release forwarded as `UiEvent::DictateKey`).

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyManager, HotKeyState};

pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    toggle: Option<HotKey>,
    enter: Option<HotKey>,
}

impl Hotkeys {
    /// Create the manager. Must be called on the main thread.
    pub fn new() -> Hotkeys {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("could not create hotkey manager: {e}");
                None
            }
        };
        Hotkeys {
            manager,
            toggle: None,
            enter: None,
        }
    }

    /// (Re)register the launcher toggle shortcut, e.g. "CmdOrCtrl+Shift+Y".
    pub fn set_toggle(&mut self, raw: &str) {
        let Some(manager) = &self.manager else {
            return;
        };
        if let Some(old) = self.toggle.take() {
            let _ = manager.unregister(old);
        }
        match parse_hotkey(raw) {
            Some(hk) => {
                if let Err(e) = manager.register(hk) {
                    eprintln!("could not register shortcut {raw}: {e}");
                } else {
                    self.toggle = Some(hk);
                }
            }
            None => eprintln!("invalid shortcut: {raw}"),
        }
    }

    /// While the dictate ring is visible, capture the global Enter key so
    /// hold-Enter-to-record works even without window focus.
    pub fn set_enter_capture(&mut self, enabled: bool) {
        let Some(manager) = &self.manager else {
            return;
        };
        if enabled {
            if self.enter.is_none() {
                let hk = HotKey::new(None::<Modifiers>, Code::Enter);
                match manager.register(hk) {
                    Ok(()) => self.enter = Some(hk),
                    Err(e) => eprintln!("could not capture global Enter: {e}"),
                }
            }
        } else if let Some(hk) = self.enter.take() {
            let _ = manager.unregister(hk);
        }
    }

    /// Drain pending hotkey events into UI actions. Call from the main
    /// thread (events are delivered there by the OS).
    pub fn drain_events(&mut self, mut on_event: impl FnMut(HotkeyEvent)) {
        use global_hotkey::GlobalHotKeyEvent;
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            let is_toggle = self.toggle.as_ref().map(|hk| hk.id()) == Some(ev.id);
            let is_enter = self.enter.as_ref().map(|hk| hk.id()) == Some(ev.id);
            match ev.state {
                HotKeyState::Pressed => {
                    if is_toggle {
                        on_event(HotkeyEvent::Toggle);
                    } else if is_enter {
                        on_event(HotkeyEvent::EnterDown);
                    }
                }
                HotKeyState::Released => {
                    if is_enter {
                        on_event(HotkeyEvent::EnterUp);
                    }
                }
            }
        }
    }
}

pub enum HotkeyEvent {
    Toggle,
    EnterDown,
    EnterUp,
}

// ---------------------------------------------------------------------------
// Shortcut string parsing ("CmdOrCtrl+Shift+Y" style)
// ---------------------------------------------------------------------------

fn parse_hotkey(raw: &str) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;

    for part in raw.split('+') {
        let p = part.trim();
        let lower = p.to_lowercase();
        match lower.as_str() {
            "cmdorctrl" | "cmd" | "command" | "super" | "meta" => {
                // Map Cmd on macOS to Ctrl elsewhere.
                if cfg!(target_os = "macos") {
                    mods |= Modifiers::META;
                } else {
                    mods |= Modifiers::CONTROL;
                }
            }
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" | "opt" => mods |= Modifiers::ALT,
            _ => {
                key = Some(parse_key(p)?);
            }
        }
    }
    let key = key?;
    Some(HotKey::new(Some(mods), key))
}

fn parse_key(p: &str) -> Option<Code> {
    let lower = p.to_lowercase();
    Some(match lower.as_str() {
        "space" => Code::Space,
        "enter" | "return" => Code::Enter,
        "tab" => Code::Tab,
        "backspace" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "up" | "arrowup" => Code::ArrowUp,
        "down" | "arrowdown" => Code::ArrowDown,
        "left" | "arrowleft" => Code::ArrowLeft,
        "right" | "arrowright" => Code::ArrowRight,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        other => {
            // F1..F24
            if let Some(num) = other.strip_prefix('f') {
                if let Ok(n) = num.parse::<u8>() {
                    if (1..=24).contains(&n) {
                        return Some(code_from_fn(n));
                    }
                }
            }
            // Single letters / digits: KeyA..KeyZ, Digit0..Digit9.
            if other.len() == 1 {
                let c = other.chars().next().unwrap();
                if c.is_ascii_alphabetic() {
                    return Some(code_from_letter(c));
                }
                if c.is_ascii_digit() {
                    return Some(code_from_digit(c));
                }
            }
            return None;
        }
    })
}

fn code_from_letter(c: char) -> Code {
    // keyboard_types Code has variants KeyA..KeyZ.
    match c {
        'a' | 'A' => Code::KeyA,
        'b' | 'B' => Code::KeyB,
        'c' | 'C' => Code::KeyC,
        'd' | 'D' => Code::KeyD,
        'e' | 'E' => Code::KeyE,
        'f' | 'F' => Code::KeyF,
        'g' | 'G' => Code::KeyG,
        'h' | 'H' => Code::KeyH,
        'i' | 'I' => Code::KeyI,
        'j' | 'J' => Code::KeyJ,
        'k' | 'K' => Code::KeyK,
        'l' | 'L' => Code::KeyL,
        'm' | 'M' => Code::KeyM,
        'n' | 'N' => Code::KeyN,
        'o' | 'O' => Code::KeyO,
        'p' | 'P' => Code::KeyP,
        'q' | 'Q' => Code::KeyQ,
        'r' | 'R' => Code::KeyR,
        's' | 'S' => Code::KeyS,
        't' | 'T' => Code::KeyT,
        'u' | 'U' => Code::KeyU,
        'v' | 'V' => Code::KeyV,
        'w' | 'W' => Code::KeyW,
        'x' | 'X' => Code::KeyX,
        'y' | 'Y' => Code::KeyY,
        'z' | 'Z' => Code::KeyZ,
        _ => Code::Unidentified,
    }
}

fn code_from_digit(c: char) -> Code {
    match c {
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => Code::Unidentified,
    }
}

fn code_from_fn(n: u8) -> Code {
    match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        _ => Code::F24,
    }
}
