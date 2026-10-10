//! The settings window — port of the React `settings.tsx`, plus editable
//! chat base URL / model (the backend supported them; the egui UI makes
//! them first-class).
//!
//! Uses the native OS window chrome (title bar + traffic-light close on
//! macOS) per user request: no custom header, no scrolling — the window
//! auto-sizes to fit its content. Styling follows app.css `.settings`:
//! glass surface (window vibrancy + dark tint from `ui::settings_frame`),
//! dim field hints and a purple-accent Save button.

use crate::app::App;
use crate::settings as settings_store;
use crate::ui::{ACCENT, ACCENT_DIM, GREEN, RED, SUBTEXT, TEXT};

/// Fixed settings window width (logical px).
const SETTINGS_W: f32 = 560.0;
/// Max window height (keeps the window on screen even with long content).
const MAX_H: f32 = 720.0;

/// Input field surface: --panel rgba(255,255,255,0.06).
const INPUT_BG: egui::Color32 = egui::Color32::from_rgba_unmultiplied_const(255, 255, 255, 15);
/// Input border: --border rgba(255,255,255,0.1).
const INPUT_BORDER: egui::Color32 = egui::Color32::from_rgba_unmultiplied_const(255, 255, 255, 26);

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    // Esc closes the window (saving first — closing must never lose edits).
    if !app.shortcut_recording && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.save_settings();
        app.close_settings(ui.ctx());
        return;
    }

    // NOTE: `draw` runs inside the root CentralPanel (app.rs applies
    // settings_frame). Nested panels don't anchor to the window edges, so
    // everything below is plain flow layout — the footer just follows the
    // content, and the window auto-sizes so it sits at the bottom.
    //
    // Everything is wrapped in one vertical child-ui whose rect is measured
    // for the auto-size. (The root panel ui's min_rect tracks the *window*
    // size, which would feed the sizer a growing value every frame.)
    let body = ui.vertical(|ui| {
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 22,
                right: 22,
                top: 14,
                bottom: 10,
            })
            .fill(egui::Color32::TRANSPARENT)
            .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 14.0;

            // ---- Global shortcut ----
            field(ui, "Global shortcut", "Press this anywhere to toggle the launcher.", |ui| {
                let label = if app.shortcut_recording {
                    "Press keys… (Esc to cancel)".to_string()
                } else {
                    let pretty = settings_store::pretty_shortcut(&app.set_shortcut);
                    if pretty.is_empty() { "Set shortcut".to_string() } else { pretty }
                };
                let btn = egui::Button::new(egui::RichText::new(label).color(
                    if app.shortcut_recording { ACCENT_DIM } else { TEXT },
                ))
                .fill(INPUT_BG)
                .stroke(egui::Stroke::new(1.0, if app.shortcut_recording { ACCENT } else { INPUT_BORDER }))
                .corner_radius(2.0)
                .min_size(egui::vec2(ui.available_width(), 34.0));
                if ui.add(btn).clicked() {
                    app.shortcut_recording = !app.shortcut_recording;
                }
            });

            // ---- AI API key ----
            field(ui, "AI — API key", "For the chat model, dictation and reminders. Default: gpt-5-mini via OpenAI.", |ui| {
                input(ui, &mut app.set_api_key, "sk-…", true);
            });

            // ---- Chat endpoint ----
            field(ui, "Chat endpoint", "Any OpenAI-compatible /chat/completions server (OpenAI, OpenRouter, Ollama…).", |ui| {
                input(ui, &mut app.set_base_url, "https://api.openai.com/v1", false);
            });

            // ---- Model (box below the label, like every other field) ----
            field(ui, "Model", "Model id used for chat and the Supacast agent.", |ui| {
                input(ui, &mut app.set_model, "gpt-5-mini", false);
            });

            // ---- Microphone status (dictate needs TCC permission) ----
            field(ui, "Microphone", "Needed for the dictate ring. If dictations come back empty, grant Supacast access.", |ui| {
                let granted = mic_permission_granted();
                ui.horizontal(|ui| {
                    let (dot, label) = match granted {
                        Some(true) => (GREEN, "Granted"),
                        Some(false) => (RED, "Denied — grant access in System Settings"),
                        None => (SUBTEXT, "Not requested yet"),
                    };
                    let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 3.5, dot);
                    ui.add(egui::Label::new(egui::RichText::new(label).color(TEXT)));
                });
                if granted == Some(false) && ui.button("Open System Settings").clicked() {
                    let _ = std::process::Command::new("open")
                        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
                        .spawn();
                }
            });

            // ---- Status ----
            // Fixed-height row for the save status. Without it the
            // "Settings updated" note appearing after Save would change the
            // content height and resize (move) the window every save.
            let status = match &app.settings_status {
                crate::app::SettingsStatus::Saved => Some(("Settings updated", GREEN)),
                crate::app::SettingsStatus::Error(e) => Some((e.as_str(), RED)),
                _ => None,
            };
            ui.allocate_ui(egui::vec2(ui.available_width(), 16.0), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if let Some((text, color)) = status {
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).size(12.5).color(color))
                            .wrap(),
                    );
                }
            });
            }); // content frame

        // ---- Footer: full-width separator, hint on the left, buttons right ----
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(
                egui::RichText::new("Saved on close — Default shortcut: ⌘ + Shift + Y")
                    .small()
                    .color(SUBTEXT),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = egui::Button::new(egui::RichText::new("Close").color(TEXT))
                .fill(INPUT_BG)
                .stroke(egui::Stroke::new(1.0, INPUT_BORDER))
                .corner_radius(2.0)
                .min_size(egui::vec2(72.0, 30.0));
            if ui.add(close).clicked() {
                app.save_settings();
                app.close_settings(ui.ctx());
            }
            let save = egui::Button::new(egui::RichText::new("Save").color(egui::Color32::WHITE))
                .fill(ACCENT)
                .stroke(egui::Stroke::NONE)
                .corner_radius(2.0)
                .min_size(egui::vec2(72.0, 30.0));
            if ui.add(save).clicked() {
                app.save_settings();
            }
        });
    });
    ui.add_space(4.0);
    }); // body vertical

    // ---- Auto-size the window to the content (no scrolling) ----
    // The root frame adds its own margin around everything drawn here.
    let h = body.response.rect.height() + 30.0;
    let h = h.min(MAX_H);
    if (h - app.settings_window_h).abs() > 1.0 {
        let first_size = app.settings_window_h == 0.0;
        app.settings_window_h = h;
        eprintln!("[settings] auto-size -> {h:.0}");
        let ctx = ui.ctx();
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(SETTINGS_W, h)));
        // Center only on the first sizing. Later content changes (the
        // "Settings updated" note after Save, error messages) must not
        // move the window — that re-centering made Save jump the window.
        if first_size {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                app.center_pos((SETTINGS_W, h)),
            ));
        }
    }

    // Capture the next key combo while recording.
    if app.shortcut_recording {
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            if let egui::Event::Key { key, modifiers, pressed: true, .. } = ev {
                if key == egui::Key::Escape {
                    app.shortcut_recording = false;
                    continue;
                }
                if matches!(
                    key,
                    egui::Key::ControlLeft
                        | egui::Key::ControlRight
                        | egui::Key::SuperLeft
                        | egui::Key::SuperRight
                        | egui::Key::AltLeft
                        | egui::Key::AltRight
                        | egui::Key::ShiftLeft
                        | egui::Key::ShiftRight
                ) {
                    continue; // modifiers alone don't complete a combo
                }
                if let Some(name) = key_name(key) {
                    let mac = cfg!(target_os = "macos");
                    let mut mods: Vec<&str> = Vec::new();
                    // macOS prefers Cmd over Ctrl; other platforms use Ctrl.
                    if (mac && modifiers.mac_cmd) || (!mac && modifiers.ctrl) {
                        mods.push(if mac { "Cmd" } else { "Ctrl" });
                    } else if modifiers.ctrl {
                        mods.push("Ctrl");
                    }
                    if modifiers.shift {
                        mods.push("Shift");
                    }
                    if modifiers.alt {
                        mods.push("Alt");
                    }
                    mods.push(name);
                    app.set_shortcut = settings_store::normalize_shortcut(&mods.join("+"));
                    app.shortcut_recording = false;
                }
            }
        }
    }
}

/// A single-line text input, styled like app.css `.api-key-input`: dark
/// panel fill, hairline border, accent border on focus.
fn input(ui: &mut egui::Ui, text: &mut String, hint: &str, password: bool) {
    let mut edit = egui::TextEdit::singleline(text)
        .hint_text(hint)
        .desired_width(ui.available_width())
        .background_color(INPUT_BG)
        .margin(egui::Margin::symmetric(8, 8))
        .font(egui::TextStyle::Body);
    if password {
        edit = edit.password(true);
    }
    let out = ui.add(edit);
    // Accent border while the field is focused (CSS :focus).
    if out.has_focus() {
        ui.painter().rect_stroke(
            out.rect,
            2.0,
            egui::Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}

/// Live macOS mic-permission status (None on other platforms / pre-request).
fn mic_permission_granted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::mic_permission_status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(true)
    }
}

/// A labelled field block: label, hint, then the control below.
fn field(
    ui: &mut egui::Ui,
    label: &str,
    hint_text: &str,
    content: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        ui.add(egui::Label::new(
            egui::RichText::new(label).size(14.0).color(TEXT),
        ));
        ui.add(egui::Label::new(
            egui::RichText::new(hint_text).size(12.5).color(SUBTEXT),
        ).wrap());
        content(ui);
    });
}

/// Map an egui Key to the name used in shortcut strings.
fn key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match key {
        A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G",
        H => "H", I => "I", J => "J", K => "K", L => "L", M => "M", N => "N",
        O => "O", P => "P", Q => "Q", R => "R", S => "S", T => "T", U => "U",
        V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4",
        Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
        F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        F13 => "F13", F14 => "F14", F15 => "F15", F16 => "F16", F17 => "F17",
        F18 => "F18", F19 => "F19", F20 => "F20",
        Space => "Space",
        Enter => "Enter",
        Tab => "Tab",
        Backspace => "Backspace",
        Delete => "Delete",
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        _ => return None,
    })
}
