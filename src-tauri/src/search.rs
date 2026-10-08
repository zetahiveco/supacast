use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    /// Display name (app name or file name).
    pub title: String,
    /// Full path (shown as subtitle in the UI).
    pub subtitle: String,
    /// What to open when the user hits Enter.
    pub path: String,
    /// "app", "file", or "folder".
    pub kind: String,
}

const MAX_APP_RESULTS: usize = 8;
const MAX_FILE_RESULTS: usize = 20;

fn matches(query: &str, name: &str) -> bool {
    name.to_lowercase().contains(&query.to_lowercase())
}

/// Rank so that items starting with the query come first.
fn score(query: &str, name: &str) -> u8 {
    if name.to_lowercase().starts_with(&query.to_lowercase()) {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------------------
// App search
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn app_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications")];
    if let Some(home) = dirs::home() {
        dirs.push(home.join("Applications"));
    }
    dirs
}

#[cfg(target_os = "windows")]
fn app_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let program_data = std::env::var("ProgramData").unwrap_or_default();
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    if !program_data.is_empty() {
        dirs.push(PathBuf::from(&program_data)
            .join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if !appdata.is_empty() {
        dirs.push(PathBuf::from(&appdata)
            .join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    dirs
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn app_search_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Recursively collect launchable entries under `dir` (macOS: `.app`
/// bundles, Windows: `.lnk`/`.exe` shortcuts) matching `query`.
fn collect_apps_in_dir(dir: &Path, query: &str, out: &mut Vec<SearchResult>) {
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .max_depth(4)
        .into_iter()
        .filter_entry(|e| !is_hidden(e));

    for entry in walker.flatten() {
        if !entry.file_type().is_file() && !is_mac_app_bundle(entry.path()) {
            continue;
        }
        let path = entry.path();
        if !is_launchable(path) {
            continue;
        }
        let name = display_name(path);
        if matches(query, &name) {
            out.push(SearchResult {
                title: name,
                subtitle: path.display().to_string(),
                path: path.display().to_string(),
                kind: "app".into(),
            });
        }
        if out.len() >= MAX_APP_RESULTS * 4 {
            break;
        }
    }
}

#[cfg(target_os = "macos")]
fn is_mac_app_bundle(path: &Path) -> bool {
    path.extension().map(|e| e == "app").unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn is_mac_app_bundle(_path: &Path) -> bool {
    false
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

/// Is this path a launchable app for the current platform?
fn is_launchable(path: &Path) -> bool {
    if is_mac_app_bundle(path) {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("lnk") | Some("exe")
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Friendly display name for an app entry.
fn display_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

pub fn search_apps(query: &str) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut results = Vec::new();
    for dir in app_search_dirs() {
        if dir.exists() {
            collect_apps_in_dir(&dir, query, &mut results);
        }
    }
    results.sort_by(|a, b| {
        (score(query, &a.title), a.title.to_lowercase()).cmp(&(
            score(query, &b.title),
            b.title.to_lowercase(),
        ))
    });
    results.truncate(MAX_APP_RESULTS);
    results
}

// ---------------------------------------------------------------------------
// File search
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn search_files(query: &str) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    // Spotlight (mdfind) does the heavy lifting; there is no limit flag,
    // so we truncate the output ourselves.
    let output = Command::new("mdfind")
        .arg("-name")
        .arg(query)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut results = Vec::new();
    for line in stdout.lines() {
        let path = PathBuf::from(line);
        if !path.exists() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();
        let is_dir = path.is_dir();
        results.push(SearchResult {
            title: name,
            subtitle: line.to_string(),
            path: line.to_string(),
            kind: if is_dir { "folder" } else { "file" }.into(),
        });
        if results.len() >= MAX_FILE_RESULTS {
            break;
        }
    }
    results
}

#[cfg(target_os = "windows")]
pub fn search_files(query: &str) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    // Walk the common user folders and match on the file name.
    // (Windows Search / Everything integration can replace this later.)
    let mut roots = Vec::new();
    if let Some(home) = dirs::home() {
        for sub in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos"] {
            let dir = home.join(sub);
            if dir.exists() {
                roots.push(dir);
            }
        }
    }

    let mut results = Vec::new();
    'outer: for root in roots {
        let walker = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .max_depth(6)
            .into_iter()
            .filter_entry(|e| !is_hidden(e));

        for entry in walker.flatten() {
            // Files and folders both count; only symlinks/oddities are skipped.
            let ft = entry.file_type();
            if !ft.is_file() && !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if matches(query, &name) {
                let path = entry.path().display().to_string();
                results.push(SearchResult {
                    title: name,
                    subtitle: path.clone(),
                    path,
                    kind: if ft.is_dir() { "folder" } else { "file" }.into(),
                });
                if results.len() >= MAX_FILE_RESULTS {
                    break 'outer;
                }
            }
        }
    }
    results
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn search_files(_query: &str) -> Vec<SearchResult> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open an app bundle / file / folder with the OS default handler.
pub fn open_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // `start "" <path>` resolves .lnk shortcuts and default handlers.
        Command::new("cmd")
            .args(["/c", "start", "", path])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Command::new("xdg-open").arg(path).spawn().map_err(|e| e.to_string())?;
    }

    Ok(())
}

// tiny cross-platform "home dir" helper (avoids adding the `dirs` crate)
#[cfg(target_os = "macos")]
mod dirs {
    pub fn home() -> Option<std::path::PathBuf> {
        std::env::var_os("HOME").map(std::path::PathBuf::from)
    }
}

#[cfg(target_os = "windows")]
mod dirs {
    pub fn home() -> Option<std::path::PathBuf> {
        std::env::var_os("USERPROFILE").map(std::path::PathBuf::from)
    }
}
