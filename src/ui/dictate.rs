//! The dictate ring — a small floating orb window. Replaces the webview
//! dictate.tsx + MediaRecorder flow with the native cpal recorder.
//!
//! Visual port of app.css `.dictate` / `.orb`: a fully transparent window
//! with a frosted-glass disc hugging a spinning conic-gradient ring around
//! a dark core. Colors/sizes follow the original CSS.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

use crate::app::App;
use crate::events::{DPhase, DictateMode};
use crate::ui::{RED, SUBTEXT, TEXT};

/// Conic-gradient stops from app.css `.orb::before`.
const RING_COLORS: [Color32; 4] = [
    Color32::from_rgb(124, 92, 255),  // #7c5cff
    Color32::from_rgb(77, 201, 255),  // #4dc9ff
    Color32::from_rgb(88, 255, 180),  // #58ffb4
    Color32::from_rgb(255, 107, 213), // #ff6bd5
];

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    // Esc cancels: stop recording (if any) and exit dictation mode.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.close_dictate(ui.ctx());
        return;
    }

    let (mode, phase, transcript, answer, error) = {
        let d = app.shared.dictate.lock().unwrap();
        (d.mode, d.phase, d.transcript.clone(), d.answer.clone(), d.error.clone())
    };

    // ---- Window surface: click-to-close when finished, drag while active ----
    let (surface_rect, surface_resp) =
        ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let _ = surface_rect;
    if surface_resp.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if surface_resp.clicked() && (phase == DPhase::Done || phase == DPhase::Error) {
        app.close_dictate(ui.ctx());
        return;
    }

    // ---- Orb (painted over the whole panel, centered) ----
    let area = ui.max_rect();
    let center = egui::pos2(area.left() + area.width() / 2.0, area.top() + area.height() / 2.0 - 14.0);

    let level = {
        let rec = app.shared.recorder.lock().unwrap();
        rec.as_ref().map(|r| r.level()).unwrap_or(0.0)
    };

    // The recording orb scales with the mic level (app.css: the JS sets
    // `scale(1 + level * 0.35)` while recording).
    let base_r = 56.0;
    let scale = if phase == DPhase::Recording { 1.0 + level * 0.35 } else { 1.0 };
    let r = base_r * scale;

    // Frosted-glass disc behind the ring (app.css `.orb::after`:
    // inset -18px, rgba(22,23,33,0.55), hairline white border).
    let p = ui.painter();
    p.circle_filled(
        center,
        base_r * scale + 18.0,
        Color32::from_rgba_unmultiplied(22, 23, 33, 140),
    );
    p.circle_stroke(
        center,
        base_r * scale + 18.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
    );

    // Ring: a spinning conic gradient (app.css `.orb::before`), slowing or
    // solidifying with the phase. done/error are static, solid rings.
    let t = ui.input(|i| i.time);
    match phase {
        DPhase::Done => {
            p.circle_stroke(center, r, Stroke::new(6.0, crate::ui::GREEN));
        }
        DPhase::Error => {
            p.circle_stroke(center, r, Stroke::new(6.0, RED));
        }
        phase => {
            let rev_per_s = match phase {
                DPhase::Recording => 1.0 / 1.1,
                DPhase::Transcribing | DPhase::Thinking => 1.0 / 0.7,
                _ => 1.0 / 3.2,
            };
            draw_conic_ring(p, center, r, 6.0, t as f32 * rev_per_s);
        }
    }

    // Dark core (app.css `.orb-core`: 100px #14151c inside the 112px orb).
    p.circle_filled(center, r - 6.0, Color32::from_rgb(20, 21, 28));

    // Core content — painted vector icons (bundled fonts render emoji
    // glyphs as tofu squares).
    let inner_rect = Rect::from_center_size(center, egui::vec2(r * 0.9, r * 0.9));
    match phase {
        DPhase::Idle => {
            // app.css `.orb-hint`: "Hold" over a kbd-style `Enter` chip.
            p.text(
                egui::pos2(center.x, center.y - 12.0),
                Align2::CENTER_CENTER,
                "Hold",
                FontId::proportional(11.0),
                SUBTEXT,
            );
            let chip = Rect::from_center_size(egui::pos2(center.x, center.y + 9.0), egui::vec2(44.0, 19.0));
            p.rect_filled(chip, 6.0, Color32::from_rgba_unmultiplied(255, 255, 255, 31));
            p.rect_stroke(chip, 6.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 51)), egui::StrokeKind::Inside);
            p.text(chip.center(), Align2::CENTER_CENTER, "Enter", FontId::proportional(11.0), TEXT);
        }
        DPhase::Listening | DPhase::Recording => {
            crate::ui::draw_mic(p, inner_rect, TEXT)
        }
        DPhase::Transcribing | DPhase::Thinking => {
            crate::ui::draw_dots(p, center, TEXT, 4.0, 12.0)
        }
        DPhase::Done => crate::ui::draw_check(p, inner_rect, crate::ui::GREEN, 3.5),
        DPhase::Error => {
            let msg = if error.chars().count() <= 32 {
                error.as_str()
            } else {
                "!"
            };
            p.text(
                center,
                Align2::CENTER_CENTER,
                msg,
                FontId::proportional(13.0),
                RED,
            );
        }
    }

    // ---- Hover ✕ on the orb's top-right corner (app.css `.orb-close`) ----
    let close_c = center + egui::vec2((r + 4.0) * 0.707 - 4.0, -(r + 4.0) * 0.707 + 4.0);
    let close_rect = Rect::from_center_size(close_c, egui::vec2(24.0, 24.0));
    let close_resp = ui.interact(close_rect, ui.id().with("orb-close"), Sense::click());
    let pointer_in = ui.rect_contains_pointer(area);
    if pointer_in || close_resp.hovered() {
        let bg = if close_resp.hovered() {
            Color32::from_rgb(232, 72, 79) // .orb-close:hover { background: #e8484f }
        } else {
            Color32::from_rgba_unmultiplied(16, 18, 26, 235)
        };
        p.circle_filled(close_c, 12.0, bg);
        p.circle_stroke(close_c, 12.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 46)));
        crate::ui::draw_x(p, close_c, 4.5, Color32::from_rgb(232, 236, 244), 1.4);
    }
    if close_resp.clicked() {
        // Cancel any in-flight recording, or just close the ring.
        if phase == DPhase::Recording || phase == DPhase::Listening {
            let _ = app.shared.events_tx.send(crate::events::UiEvent::StopRecording(true));
        } else {
            app.close_dictate(ui.ctx());
        }
        return;
    }

    // Mode tag under the orb.
    let mode_label = match mode {
        DictateMode::Text => "Text → clipboard",
        DictateMode::Supacast => "Ask Supacast",
    };
    p.text(
        egui::pos2(center.x, center.y + r + 28.0),
        Align2::CENTER_CENTER,
        mode_label,
        FontId::proportional(11.0),
        SUBTEXT,
    );

    // Transcript, centered under the orb like the React `.dictate-transcript`
    // (single line, clipped).
    if !transcript.is_empty() {
        p.text(
            egui::pos2(center.x, center.y + r + 48.0),
            Align2::CENTER_CENTER,
            &format!("“{transcript}”"),
            FontId::proportional(13.0),
            TEXT,
        );
    }

    // Agent answer / long error below.
    let mut y = area.bottom() - 16.0;
    if !answer.is_empty() {
        y = draw_wrapped(ui, Pos2::new(area.left() + 12.0, y), &answer, SUBTEXT);
    }
    if phase == DPhase::Error && error.chars().count() > 32 {
        draw_wrapped(ui, Pos2::new(area.left() + 12.0, y), &error, RED);
    }
}

/// Draw the spinning conic-gradient ring: `segments` short arc strokes whose
/// colors sweep through `RING_COLORS`. `turns_per_s` rotates it; it reads
/// as a smooth gradient at this segment count.
fn draw_conic_ring(
    p: &egui::Painter,
    center: Pos2,
    r: f32,
    width: f32,
    turns_per_s: f32,
) {
    const TAU: f32 = std::f32::consts::TAU;
    let segments = 60;
    let rot = turns_per_s * TAU; // continuous rotation (i.time is monotonic)
    for i in 0..segments {
        let f0 = i as f32 / segments as f32;
        let f1 = (i + 1) as f32 / segments as f32;
        let a0 = rot + f0 * TAU;
        let a1 = rot + f1 * TAU + 0.02; // slight overlap: no gaps
        let color = gradient_at(f0);
        let p0 = Pos2::new(center.x + r * a0.cos(), center.y + r * a0.sin());
        let p1 = Pos2::new(center.x + r * a1.cos(), center.y + r * a1.sin());
        p.line_segment([p0, p1], Stroke::new(width, color));
    }
}

/// Position on the 4-stop conic gradient (wrapping).
fn gradient_at(t: f32) -> Color32 {
    let t = t - t.floor();
    let stops = RING_COLORS;
    let pos = t * stops.len() as f32;
    let i = (pos as usize) % stops.len();
    let j = (i + 1) % stops.len();
    let frac = pos - pos.floor();
    lerp(stops[i], stops[j], frac)
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// Draw wrapped text starting at `top_left`; returns the y where the next
/// element should start (text is painted via a Label-style galley).
fn draw_wrapped(ui: &mut egui::Ui, top_left: Pos2, text: &str, color: Color32) -> f32 {
    let max_w = ui.available_width().max(1.0) - 24.0;
    let galley = {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = max_w;
        job.append(text, 0.0, egui::TextFormat::simple(FontId::proportional(12.0), color));
        ui.painter().layout_job(job)
    };
    let h = galley.size().y;
    let y0 = top_left.y - h;
    if y0 > 120.0 {
        ui.painter().galley(egui::pos2(top_left.x, y0), galley, color);
        y0
    } else {
        top_left.y
    }
}
