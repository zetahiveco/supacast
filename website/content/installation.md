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
[GitHub Releases](https://github.com/zetahiveco/supacast/releases) and installs
**Supacast.app** into `/Applications` (falling back to `~/Applications` if
needed), with the rocket app icon. It also symlinks a `supacast` command into
`/usr/local/bin` (or `~/.local/bin`) so you can launch it from the terminal,
and strips any quarantine flags so it opens normally.

### Windows (PowerShell)

Open PowerShell and run:

```powershell
powershell -c "irm https://supacast-omega.vercel.app/install.ps1 | iex"
```

This downloads the latest Windows archive from GitHub Releases, extracts it to
`%LOCALAPPDATA%\Programs\Supacast`, adds it to your user PATH and registers
Supacast to start automatically when Windows boots.

## Verify the install

After installing:

1. Look for the **rocket icon 🚀** in the menu bar (macOS) or system tray (Windows/Linux).
2. Press `⌘ + ⇧ + Y` (macOS) or `Ctrl + ⇧ + Y` (Windows) to summon the launcher.
3. If the shortcut doesn't respond, open Supacast once from
   the terminal (`supacast`) or the Start Menu (Windows) so it can register its
   global shortcut and enable launch-at-login.

## Manual install

Prefer to install by hand? Grab the right asset from the
[latest release](https://github.com/zetahiveco/supacast/releases):

| Platform | Asset | Action |
|----------|-------|--------|
| macOS (Apple Silicon) | `supacast-aarch64-apple-darwin.tar.gz` | Extract the binary into a `Supacast.app` bundle in `/Applications` (the one-line install builds this for you) |
| macOS (Intel) | `supacast-x86_64-apple-darwin.tar.gz` | Same as above |
| Windows | `supacast-x86_64-pc-windows-msvc.zip` | Extract `supacast.exe` anywhere on your PATH |
| Linux | `supacast-x86_64-unknown-linux-gnu.tar.gz` | Extract and move `supacast` to `/usr/local/bin` |

> **Seeing "Supacast is damaged" on macOS?** Browsers stamp downloads with a
> quarantine flag — remove the flag and it opens normally:
>
> ```sh
> xattr -cr /Applications/Supacast.app
> ```
>
> The `curl | bash` one-line install does this for you automatically.

## Uninstall

- **macOS**: quit from the tray menu, then drag `Supacast.app` from
  `/Applications` to the Trash. Also delete the `supacast` symlink from
  `/usr/local/bin` (if present) and
  `~/Library/LaunchAgents/com.supacast.app.plist` to stop launch-at-login.
- **Windows**: delete `%LOCALAPPDATA%\Programs\Supacast`, remove the
  `Supacast` entry from the registry `Run` key and your user PATH.
- **Linux**: delete the `supacast` binary from `/usr/local/bin` or `~/.local/bin`.

## Build from source

Prerequisites: the [Rust toolchain](https://rustup.rs).

```sh
git clone https://github.com/zetahiveco/supacast.git
cd supacast
cargo run                # run in development
cargo build --release    # build an optimized binary
```

On Linux you also need the dev packages
(`sudo apt install libgtk-3-dev libasound2-dev libdbus-1-dev libxkbcommon-dev`).
The binary lands in `target/release/supacast`.
