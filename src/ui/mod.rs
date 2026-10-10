//! Shared UI helpers + the three window modules.

pub mod dictate;
pub mod launcher;
pub mod settings;

use egui::Color32;

// Dark, launcher-style palette — straight from the original app.css.
// Translucent fills are intentional: the real backdrop blur comes from the
// NSVisualEffectView (window vibrancy) behind the window.
pub const BG: Color32 = Color32::from_rgba_unmultiplied_const(24, 26, 32, 255);
/// Launcher surface: rgba(24, 26, 34, 0.45) over the vibrancy layer.
pub const GLASS_LAUNCHER: Color32 = Color32::from_rgba_unmultiplied_const(24, 26, 34, 115);
/// Settings surface: rgba(18, 20, 26, 0.68) — darker for text readability.
pub const GLASS_SETTINGS: Color32 = Color32::from_rgba_unmultiplied_const(18, 20, 26, 173);
/// Window border: rgba(255, 255, 255, 0.14).
pub const BORDER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 36);
/// Rows: --panel-active rgba(124, 92, 255, 0.25).
pub const ROW_ACTIVE: Color32 = Color32::from_rgba_unmultiplied_const(124, 92, 255, 64);
pub const ROW_HOVER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 18);
pub const TEXT: Color32 = Color32::from_rgb(242, 243, 247); // #f2f3f7
pub const SUBTEXT: Color32 = Color32::from_rgba_unmultiplied_const(242, 243, 247, 140);
pub const ACCENT: Color32 = Color32::from_rgb(124, 92, 255); // #7c5cff
pub const ACCENT_DIM: Color32 = Color32::from_rgb(201, 186, 255); // #c9baff
pub const GREEN: Color32 = Color32::from_rgb(88, 255, 180); // #58ffb4
pub const RED: Color32 = Color32::from_rgb(255, 123, 123); // #ff7b7b
pub const PURPLE: Color32 = Color32::from_rgb(190, 140, 250);

/// The frosted-glass launcher surface (vibrancy does the blur; this adds
/// the dark tint + hairline border from app.css `.launcher`).
pub fn launcher_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(GLASS_LAUNCHER)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(0.0) // sharp edges
        .inner_margin(egui::Margin::same(12))
}

/// Same idea for the settings window (app.css `.settings`).
pub fn settings_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(GLASS_SETTINGS)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(0.0)
        .inner_margin(egui::Margin::same(12))
}

/// One mixed-style line: `title` in text color + `subtitle` dimmed.
#[allow(dead_code)] // kept for future views
pub fn title_subtitle(title: &str, subtitle: &str, width: f32) -> egui::widget_text::WidgetText {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: width,
            ..Default::default()
        },
        ..Default::default()
    };
    job.append(
        title,
        0.0,
        egui::TextFormat::simple(egui::FontId::proportional(14.0), TEXT),
    );
    if !subtitle.is_empty() {
        job.append(
            &format!("   {subtitle}"),
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(11.0), SUBTEXT),
        );
    }
    egui::widget_text::WidgetText::LayoutJob(std::sync::Arc::new(job))
}

// ---------------------------------------------------------------------------
// Vector icons — drawn with the painter. The bundled egui fonts don't cover
// emoji/symbol glyphs (they render as tofu squares), so every icon in the
// app is painted.
// ---------------------------------------------------------------------------

/// A small ✕ (close/delete) icon button painted with line strokes.
#[allow(dead_code)] // kept for future views (the settings window now uses native chrome)
pub fn icon_x(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 0.0, ROW_ACTIVE); // sharp
    }
    draw_x(p, rect.center(), 5.0, SUBTEXT, 1.8);
    resp
}

/// A ✕ drawn from two line segments centered at `center`.
pub fn draw_x(
    p: &egui::Painter,
    center: egui::Pos2,
    r: f32,
    color: Color32,
    width: f32,
) {
    let stroke = egui::Stroke::new(width, color);
    p.line_segment(
        [center - egui::vec2(r, r), center + egui::vec2(r, r)],
        stroke,
    );
    p.line_segment(
        [center + egui::vec2(r, -r), center + egui::vec2(-r, r)],
        stroke,
    );
}

/// A ✓ check mark inside `rect` (used by checkboxes and the dictate orb).
pub fn draw_check(p: &egui::Painter, rect: egui::Rect, color: Color32, width: f32) {
    let pts = [
        egui::pos2(rect.left() + rect.width() * 0.22, rect.center().y + rect.height() * 0.08),
        egui::pos2(rect.center().x - rect.width() * 0.02, rect.bottom() - rect.height() * 0.22),
        egui::pos2(rect.right() - rect.width() * 0.18, rect.top() + rect.height() * 0.22),
    ];
    p.add(egui::Shape::line(pts.to_vec(), egui::Stroke::new(width, color)));
}

/// A ▶ play triangle centered in `rect`.
pub fn draw_play(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let c = rect.center();
    let s = rect.height() * 0.30;
    let pts = [
        egui::pos2(c.x - s * 0.7, c.y - s),
        egui::pos2(c.x - s * 0.7, c.y + s),
        egui::pos2(c.x + s * 1.1, c.y),
    ];
    p.add(egui::Shape::convex_polygon(
        pts.to_vec(),
        color,
        egui::Stroke::NONE,
    ));
}

/// A ⏸ pause icon (two bars) centered in `rect`.
pub fn draw_pause(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let c = rect.center();
    let w = rect.width() * 0.22;
    let h = rect.height() * 0.62;
    p.rect_filled(
        egui::Rect::from_center_size(egui::pos2(c.x - w, c.y), egui::vec2(w, h)),
        0.0,
        color,
    );
    p.rect_filled(
        egui::Rect::from_center_size(egui::pos2(c.x + w, c.y), egui::vec2(w, h)),
        0.0,
        color,
    );
}

/// Three pulsing dots (thinking/transcribing) centered at `center`.
/// Animate by varying `radius`/`spacing` from the caller over time.
pub fn draw_dots(p: &egui::Painter, center: egui::Pos2, color: Color32, radius: f32, spacing: f32) {
    for i in [-1.0f32, 0.0, 1.0] {
        p.circle_filled(egui::pos2(center.x + i * spacing, center.y), radius, color);
    }
}

/// A 🔍 magnifying-glass icon (search) centered in `rect`.
pub fn draw_search(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let c = rect.center();
    let r = rect.height() * 0.28;
    let glass_c = egui::pos2(c.x - r * 0.25, c.y - r * 0.25);
    p.circle_stroke(glass_c, r, egui::Stroke::new(2.0, color));
    let handle_start = glass_c + egui::vec2(r * 0.72, r * 0.72);
    let handle_end = glass_c + egui::vec2(r * 1.55, r * 1.55);
    p.line_segment([handle_start, handle_end], egui::Stroke::new(2.2, color));
}

/// A 🎙 microphone icon centered in `rect`.
pub fn draw_mic(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let c = rect.center();
    let body_w = rect.width() * 0.36;
    let body_h = rect.height() * 0.46;
    let body = egui::Rect::from_center_size(
        egui::pos2(c.x, c.y - rect.height() * 0.10),
        egui::vec2(body_w, body_h),
    );
    p.rect_filled(body, body_w / 2.0, color);
    // U-shaped holder: two short strokes down the sides + a bottom bar.
    let holder_r = body_w * 0.95;
    p.line_segment(
        [
            egui::pos2(c.x - holder_r, body.center().y),
            egui::pos2(c.x - holder_r, body.bottom()),
        ],
        egui::Stroke::new(1.6, color),
    );
    p.line_segment(
        [
            egui::pos2(c.x + holder_r, body.center().y),
            egui::pos2(c.x + holder_r, body.bottom()),
        ],
        egui::Stroke::new(1.6, color),
    );
    p.line_segment(
        [
            egui::pos2(c.x - holder_r, body.bottom()),
            egui::pos2(c.x + holder_r, body.bottom()),
        ],
        egui::Stroke::new(1.6, color),
    );
    // Stem down from the holder.
    p.line_segment(
        [
            egui::pos2(c.x, body.bottom()),
            egui::pos2(c.x, body.bottom() + rect.height() * 0.16),
        ],
        egui::Stroke::new(1.6, color),
    );
}
