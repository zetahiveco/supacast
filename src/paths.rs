use std::path::PathBuf;

/// Application data directory, mirroring the Tauri app's identifier so
/// existing data (todos, notes, dictations, settings) carries over.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("com.supacast.app"),
        )
    } else if cfg!(target_os = "windows") {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("com.supacast.app"))
    } else {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })?;
        Some(base.join("supacast"))
    }
}

/// Helper for data-file paths (todos, clipboard history, ...).
pub fn data_file(name: &str) -> Option<PathBuf> {
    data_dir().map(|d| d.join(name))
}
