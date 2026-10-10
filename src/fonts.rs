//! System font integration — loads the OS UI font (SF Pro on macOS) as the
//! primary proportional font, keeping egui's bundled fonts (Ubuntu-Light,
//! Hack, NotoEmoji, emoji-icon-font) as fallbacks.

use std::sync::Arc;

/// Install the system font into the egui context. Falls back silently to
/// the bundled fonts if no system font file can be read.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Candidate system fonts, in preference order.
    #[cfg(target_os = "macos")]
    let candidates: &[&str] = &[
        "/System/Library/Fonts/SFNS.ttf",    // SF Pro (system UI font)
        "/System/Library/Fonts/SFNSRounded.ttf",
        "/System/Library/Fonts/SFCompact.ttf",
    ];
    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &[
        "C:\\Windows\\Fonts\\segoeui.ttf",   // Segoe UI
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let candidates: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ];

    for path in candidates {
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts.font_data.insert(
                    "system".to_owned(),
                    Arc::new(egui::FontData::from_owned(bytes)),
                );
                // System font first; bundled Ubuntu-Light & co. become
                // fallbacks for anything it lacks (⌘, emoji, etc.).
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "system".to_owned());
                break;
            }
            Err(_) => continue,
        }
    }

    ctx.set_fonts(fonts);
}
