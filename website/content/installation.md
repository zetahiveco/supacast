+++
title = "Installation"
description = "Install Supacast on macOS, Windows or Linux with a single command."
+++

## One-line install

### macOS / Linux

Open a terminal and run:

```sh
curl -fsSL https://supacast-omega.vercel.app/install.sh | bash
```

The script downloads the latest release from
[GitHub Releases](https://github.com/zetahiveco/supacast/releases), copies
`Supacast.app` into `/Applications` (or installs the `.deb` / AppImage on
Linux), and Supacast registers itself to **launch automatically at every
login**.

### Windows (PowerShell)

Open PowerShell and run:

```powershell
powershell -c "irm https://supacast-omega.vercel.app/install.ps1 | iex"
```

This downloads the latest NSIS installer from GitHub Releases and runs it
silently. Afterwards you'll find Supacast in your Start Menu and system tray,
and it will start automatically when Windows boots.

## Verify the install

After installing:

1. Look for the **rocket icon 🚀** in the menu bar (macOS) or system tray (Windows/Linux).
2. Press `⌘ + ⇧ + Y` (macOS) or `Ctrl + ⇧ + Y` (Windows) to summon the launcher.
3. If the shortcut doesn't respond, open Supacast once from
   `/Applications` (macOS) or the Start Menu (Windows) so it can register its
   global shortcut and enable launch-at-login.

## Manual install

Prefer to install by hand? Grab the right asset from the
[latest release](https://github.com/zetahiveco/supacast/releases):

| Platform | Asset | Action |
|----------|-------|--------|
| macOS (Apple Silicon) | `Supacast_aarch64.app.tar.gz` or `.dmg` | Extract to `/Applications` |
| macOS (Intel) | `Supacast_x64.app.tar.gz` or `.dmg` | Extract to `/Applications` |
| Windows | `Supacast_x64-setup.exe` | Run the NSIS installer |
| Linux | `Supacast_amd64.deb` | `sudo dpkg -i Supacast_amd64.deb` |
| Linux | `Supacast_amd64.AppImage` | `chmod +x` and run |

> **Seeing "Supacast is damaged" on macOS?** Browsers stamp downloads with a
> quarantine flag, and the app isn't notarized yet — remove the flag and it
> opens normally:
>
> ```sh
> xattr -cr /Applications/Supacast.app
> ```
>
> The `curl | bash` one-line install does this for you automatically.

## Uninstall

- **macOS**: quit from the tray menu, then drag `Supacast.app` out of
  `/Applications`. Remove `~/Library/LaunchAgents/com.supacast.app.plist` to
  stop launch-at-login.
- **Windows**: uninstall from *Settings → Apps*, or re-run the installer and
  choose uninstall.
- **Linux**: `sudo dpkg -r supacast` (deb), or delete the AppImage.

## Build from source

Prerequisites: [Node.js](https://nodejs.org) 18+ and the
[Rust toolchain](https://rustup.rs).

```sh
git clone https://github.com/zetahiveco/supacast.git
cd supacast
npm install
npm run tauri dev    # run in development
npm run tauri build  # build a distributable bundle
```

Bundles land in `src-tauri/target/release/bundle/`.