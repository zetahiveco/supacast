# Supacast

<p align="center">
  <strong>A fast, keyboard-first launcher for apps, files &amp; AI — Spotlight/Raycast style.</strong>
</p>

<p align="center">
  <a href="https://supacast-omega.vercel.app">Website</a> ·
  <a href="https://supacast-omega.vercel.app/installation/">Install</a> ·
  <a href="https://supacast-omega.vercel.app/docs/">Docs</a> ·
  <a href="https://github.com/zetahiveco/supacast/releases">Releases</a>
</p>

Supacast is a cross-platform launcher built with **Tauri 2 + React + TypeScript**.
It lives quietly in your menu bar / system tray, opens with a single global
shortcut, and lets you search and launch apps, files, todos, notes, clipboard
history and calendar events — plus an AI agent you can chat with.

## Features

- 🚀 Lives in the **menu bar / system tray** (rocket icon) — no dock icon on macOS
- 🔍 **Global app search** — macOS `.app` bundles, Windows Start Menu shortcuts
- 📁 **Global file search** — macOS Spotlight (`mdfind`), Windows user-folder scan
- ✅ **Todos & reminders** with due dates and background notifications
- 📝 **Notes** with instant search
- 📋 **Clipboard history** — everything you copy is one search away
- 📅 **Calendar** — view and create events straight from the launcher
- 🎙️ **Dictate** — voice-to-text with paste into any app (or straight into the agent)
- 🤖 **AI agent** — multi-turn chat with streamed output (bring your own API key)
- ⌨️ Global shortcut **`⌘ + ⇧ + Y`** (macOS) / **`Ctrl + ⇧ + Y`** (Windows) —
  customizable from Settings
- 🔄 **Launch at login** — starts automatically with your system after install
- Blur-to-hide, keyboard-first navigation (↑↓ navigate, ↵ open, Esc hide)

## Install

### macOS / Linux

```sh
curl -fsSL https://supacast-omega.vercel.app/install.sh | bash
```

### Windows (PowerShell)

```powershell
powershell -c "irm https://supacast-omega.vercel.app/install.ps1 | iex"
```

The scripts download the latest release from
[GitHub Releases](https://github.com/zetahiveco/supacast/releases), install it
(`/Applications` on macOS, the NSIS installer on Windows), and Supacast will
launch automatically every time your system boots.

See the [installation guide](https://supacast-omega.vercel.app/installation/) for
manual installs and Linux packages.

## Build from source

Prerequisites: [Node.js](https://nodejs.org) 18+ and the
[Rust toolchain](https://rustup.rs).

```sh
npm install
npm run tauri dev    # run in development
npm run tauri build  # build a distributable bundle
```

## Documentation

Full docs — shortcuts, settings, dictate, the AI agent and troubleshooting —
live at [supacast.vercel.app/docs](https://supacast-omega.vercel.app/docs/).

## Settings

Stored in the OS app-data directory (`settings.json`) and editable from the
tray menu → **Settings**. The default shortcut uses `CmdOrCtrl+Shift+Y`, which
maps to Cmd on macOS and Ctrl on Windows automatically.

## License

[MIT](./LICENSE) © Harish Deivanayagam

---

Made by [harishdeivanayagam](https://github.com/harishdeivanayagam) · [zetahive.co](https://zetahive.co)