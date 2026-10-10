//! Verify `decode_audio` handles every recording in the dictations store:
//! the hand-rolled WAV path (this app) and the symphonia path (m4a from the
//! old Tauri app). Prints one line per recording.

use std::path::PathBuf;

fn main() {
    let dir = match directories() {
        Some(d) => d.join("dictations"),
        None => {
            eprintln!("no app data dir");
            std::process::exit(1);
        }
    };
    let meta = match std::fs::read_to_string(dir.parent().unwrap().join("dictations.json")) {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_default(),
        Err(e) => {
            eprintln!("no dictations.json: {e}");
            std::process::exit(1);
        }
    };
    for d in meta.as_array().unwrap() {
        let id = d["id"].as_str().unwrap_or_default();
        let mime = d["mime"].as_str().unwrap_or_default();
        let ext = if mime.contains("wav") {
            "wav"
        } else if mime.contains("mp4") {
            "m4a"
        } else {
            "webm"
        };
        let path = PathBuf::from(&dir).join(format!("{id}.{ext}"));
        let bytes = std::fs::read(&path).unwrap_or_default();
        match supacast::audio::decode_audio(&bytes) {
            Ok((samples, rate)) => {
                println!(
                    "OK   {} ({mime}): {} samples @ {rate} Hz = {:.1}s",
                    &id[..8],
                    samples.len(),
                    samples.len() as f64 / rate as f64
                );
            }
            Err(e) => println!("FAIL {} ({mime}): {e}", &id[..8]),
        }
    }
}

/// Same location the app uses (`paths::data_dir`): ~/Library/Application
/// Support/com.supacast.app on macOS.
fn directories() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    #[cfg(target_os = "macos")]
    let base = format!("{home}/Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = format!("{home}/.local/share");
    Some(PathBuf::from(base).join("com.supacast.app"))
}
