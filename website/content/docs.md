+++
title = "Docs"
description = "Shortcuts, features, settings and troubleshooting for Supacast."
+++

## The basics

Supacast lives in your menu bar / system tray as a **rocket icon 🚀** and stays
out of the way. Summon it with the global shortcut, search, press ↵ to launch,
press Esc or click away to hide it.

| Key | Action |
|-----|--------|
| `⌘ + ⇧ + Y` / `Ctrl + ⇧ + Y` | Show / hide the launcher (customizable) |
| `↑` / `↓` | Navigate results |
| `↵` | Open the selected result |
| `Esc` | Hide the launcher |

## Launcher modes

Supacast's search bar handles several kinds of input:

- **Apps** — type to search installed applications (macOS `.app` bundles,
  Windows Start Menu shortcuts). ↵ opens the top result.
- **Files** — search files via macOS Spotlight (`mdfind`) or the Windows
  user-folder scan.
- **Todos** — add with a due date (`/todo Buy milk tomorrow 5pm`) and get a
  notification when it's time.
- **Notes** — quick notes with instant full-text search.
- **Clipboard history** — everything you copy is saved; search and re-copy it.
- **Calendar** — view today's events and create new ones
  (`/event Standup tomorrow 9am 30m`).

## Dictate

Open **Dictate** from the tray menu (or the launcher) and speak instead of
typing. Two modes:

- **Text** — the transcript is copied to your clipboard and pasted directly
  into the app that had focus before you started (requires the Accessibility
  permission on macOS).
- **Supacast** — the transcript is answered by the AI agent and shown in the
  launcher window.

## AI agent

The agent supports multi-turn conversations with streamed output. To enable
it, open **Settings** from the tray menu and set:

- **OpenAI API key** (required) — stored locally in your app-data directory,
  never sent anywhere except the endpoint you configure.
- **Base URL** (optional) — point it at any OpenAI-compatible endpoint
  (e.g. a local model server or a different provider).
- **Model** (optional) — pick the model the agent and dictation use.

## Settings

Open **Settings** from the tray menu. Settings live in
`settings.json` inside the OS app-data directory:

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/com.supacast.app/settings.json` |
| Windows | `%APPDATA%\com.supacast.app\settings.json` |
| Linux | `~/.config/com.supacast.app/settings.json` |

## Launch at login

Supacast always starts automatically when your system boots — installed once,
always available. It registers itself with the OS at every launch:

- **macOS** — a launch agent is registered under
  `~/Library/LaunchAgents/com.supacast.app.plist`.
- **Windows** — a registry `Run` entry is created for the current user.

Supacast starts hidden in the tray, so it won't steal focus at boot.

## Troubleshooting

**Global shortcut doesn't work.** Open Supacast once manually after install
so it can register the shortcut, then check for conflicts with another app
that owns `⌘⇧Y`.

**Dictate paste doesn't land in the other app.** On macOS, grant Supacast
*Accessibility* access in *System Settings → Privacy & Security →
Accessibility*. Without it, the transcript still lands on your clipboard.

**File search returns nothing on macOS.** Spotlight permissions: check
*System Settings → Siri & Spotlight* and make sure the relevant folders are
indexed.

**Wrong app version shows in the tray.** Use the tray menu → *Quit Supacast*,
then reopen it — the single-instance handler shows the already-running copy
otherwise.

---

More questions? Open an issue on
[GitHub](https://github.com/zetahiveco/supacast/issues).