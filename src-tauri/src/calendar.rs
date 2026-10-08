use chrono::{DateTime, Local, NaiveDate, TimeZone};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CalendarEvent {
    pub title: String,
    /// RFC3339 start time (local offset).
    pub start: String,
    /// RFC3339 end time, when known.
    pub end: Option<String>,
}

// ---------------------------------------------------------------------------
// macOS — Calendar.app via AppleScript (osascript), no extra deps.
// Dates cross the boundary as epoch seconds to dodge locale parsing.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use super::CalendarEvent;
    use chrono::{DateTime, Local};
    use std::process::Command;

    fn run_apple_script(script: &str) -> Result<String, String> {
        let out = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("failed to run osascript: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn add_event(
        title: &str,
        start: DateTime<Local>,
        duration_min: i64,
    ) -> Result<(), String> {
        let epoch = start.timestamp();
        let script = format!(
            r#"
set startEpoch to {epoch}
set dur to {duration_min}
set t to "{title}"
set sd to current date
set time of sd to 0
set sd to sd + startEpoch
set ed to sd + (dur * minutes)
tell application "Calendar"
    set cal to first calendar whose writable is true
    if cal is missing value then set cal to first calendar
    make new event at end of events of cal with properties {{summary:t, start date:sd, end date:ed}}
end tell
"#
        );
        run_apple_script(&script)?;
        Ok(())
    }

    pub fn list_events(
        day_start: DateTime<Local>,
        day_end: DateTime<Local>,
    ) -> Result<Vec<CalendarEvent>, String> {
        let e1 = day_start.timestamp();
        let e2 = day_end.timestamp();
        let script = format!(
            r#"
set out to ""
set d1 to current date
set time of d1 to 0
set d1 to d1 + {e1}
set d2 to current date
set time of d2 to 0
set d2 to d2 + {e2}
tell application "Calendar"
    repeat with c in calendars
        try
            set evs to (every event of c whose start date is greater than or equal to d1 and start date is less than d2)
            repeat with e in evs
                set out to out & (summary of e) & "|" & ((start date of e) as string) & linefeed
            end repeat
        end try
    end repeat
end tell
return out
"#
        );
        let raw = run_apple_script(&script)?;

        let mut events = Vec::new();
        for line in raw.lines() {
            let mut parts = line.splitn(2, '|');
            let title = parts.next().unwrap_or("").trim();
            if title.is_empty() {
                continue;
            }
            events.push(CalendarEvent {
                title: title.to_string(),
                start: parts.next().unwrap_or("").trim().to_string(),
                end: None,
            });
        }
        Ok(events)
    }
}

// ---------------------------------------------------------------------------
// Windows — Outlook COM via PowerShell (native Outlook calendar).
// Data crosses via env vars to dodge quoting hell.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod imp {
    use super::CalendarEvent;
    use chrono::{DateTime, Local};
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn run_powershell(script: &str, envs: &[(&str, String)]) -> Result<String, String> {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().map_err(|e| format!("failed to run powershell: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn add_event(title: &str, start: DateTime<Local>, duration_min: i64) -> Result<(), String> {
        let start_str = start.format("%Y-%m-%d %H:%M").to_string();
        run_powershell(
            r#"
$o = New-Object -ComObject Outlook.Application
$e = $o.CreateItem(1)  # olAppointmentItem
$e.Subject = $env:SC_TITLE
$e.Start = $env:SC_START
$e.Duration = [int]$env:SC_DUR
$e.Save()
"#,
            &[
                ("SC_TITLE", title.to_string()),
                ("SC_START", start_str),
                ("SC_DUR", duration_min.to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn list_events(day_start: DateTime<Local>, day_end: DateTime<Local>) -> Result<Vec<CalendarEvent>, String> {
        let s1 = day_start.format("%Y-%m-%d %H:%M").to_string();
        let s2 = day_end.format("%Y-%m-%d %H:%M").to_string();
        let raw = run_powershell(
            r#"
$o = New-Object -ComObject Outlook.Application
$ns = $o.GetNamespace("MAPI")
$cal = $ns.GetDefaultFolder(9)  # olFolderCalendar
$items = $cal.Items
$items.IncludeRecurrences = $true
$items.Sort("[Start]")
$restricted = $items.Restrict("[Start] >= '" + $env:SC_FROM + "' And [Start] < '" + $env:SC_TO + "'")
foreach ($e in $restricted) {
    Write-Output ("{0}|{1}|{2}" -f $e.Subject, $e.Start.ToString("yyyy-MM-ddTHH:mm"), $e.End.ToString("yyyy-MM-ddTHH:mm"))
}
"#,
            &[("SC_FROM", s1), ("SC_TO", s2)],
        )?;

        let mut events = Vec::new();
        for line in raw.lines() {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.is_empty() || parts[0].trim().is_empty() {
                continue;
            }
            events.push(CalendarEvent {
                title: parts[0].trim().to_string(),
                start: parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default(),
                end: parts.get(2).map(|s| s.trim().to_string()),
            });
        }
        Ok(events)
    }
}

// ---------------------------------------------------------------------------
// Other platforms: unsupported
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::CalendarEvent;
    use chrono::{DateTime, Local};
    pub fn add_event(_t: &str, _s: DateTime<Local>, _d: i64) -> Result<(), String> {
        Err("calendar is not supported on this platform".into())
    }
    pub fn list_events(_a: DateTime<Local>, _b: DateTime<Local>) -> Result<Vec<CalendarEvent>, String> {
        Err("calendar is not supported on this platform".into())
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn add_event(title: &str, start_iso: &str, duration_min: i64) -> Result<CalendarEvent, String> {
    let start = parse_when(start_iso)?;
    imp::add_event(title, start, duration_min)?;
    Ok(CalendarEvent {
        title: title.to_string(),
        start: start.to_rfc3339(),
        end: Some((start + chrono::Duration::minutes(duration_min)).to_rfc3339()),
    })
}

/// scope: "today" | "tomorrow"
pub fn list_events(scope: &str) -> Result<Vec<CalendarEvent>, String> {
    let today = Local::now().date_naive();
    let day = match scope {
        "tomorrow" => today.succ_opt().unwrap_or(today),
        _ => today,
    };
    let start = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .ok_or("bad local time")?;
    let end = Local
        .from_local_datetime(
            &day.succ_opt()
                .unwrap_or(day)
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .single()
        .ok_or("bad local time")?;
    let mut events = imp::list_events(start, end)?;
    events.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(events)
}

/// Parse flexible "when" strings coming from the agent or UI.
pub fn parse_when(raw: &str) -> Result<DateTime<Local>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty date".into());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.with_timezone(&Local));
    }
    for fmt in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%d"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Local
                .from_local_datetime(&dt)
                .single()
                .ok_or_else(|| "ambiguous local time".to_string());
        }
        if let Ok(d) = NaiveDate::parse_from_str(raw, fmt) {
            if let Some(ndt) = d.and_hms_opt(9, 0, 0) {
                return Local
                    .from_local_datetime(&ndt)
                    .single()
                    .ok_or_else(|| "ambiguous local time".to_string());
            }
        }
    }
    Err(format!("could not parse date: {raw}"))
}
