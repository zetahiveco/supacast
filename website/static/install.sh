#!/usr/bin/env bash
# Supacast installer — macOS & Linux
#
# Usage: curl -fsSL https://supacast-omega.vercel.app/install.sh | bash
set -euo pipefail

REPO="zetahiveco/supacast"
APP_NAME="Supacast"

info()  { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
error() { printf '\033[1;31mError:\033[0m %s\n' "$1" >&2; exit 1; }

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) PLATFORM="darwin" ;;
  Linux)  PLATFORM="linux" ;;
  *) error "Unsupported OS '$OS'. On Windows use: powershell -c \"irm https://supacast-omega.vercel.app/install.ps1 | iex\"" ;;
esac

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

info "Fetching the latest ${APP_NAME} release..."
RELEASE_JSON="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")" \
  || error "Could not reach GitHub Releases. Check your network connection."

# Grab every download URL from the release payload.
ASSET_URLS="$(printf '%s' "$RELEASE_JSON" | grep -oE '"browser_download_url":\s*"[^"]+"' | cut -d'"' -f4)"
[ -n "$ASSET_URLS" ] || error "No release assets found."

# Release version (for the app bundle's version fields), e.g. v0.2.1 -> 0.2.1.
TAG="$(printf '%s' "$RELEASE_JSON" | grep -m1 -oE '"tag_name":\s*"[^"]+"' | cut -d'"' -f4)"
VERSION="${TAG#v}"

ASSET_URL=""
if [ "$PLATFORM" = "darwin" ]; then
  if [ "$ARCH" = "arm64" ]; then
    ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'aarch64-apple-darwin' || true)"
  else
    ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'x86_64-apple-darwin' || true)"
  fi
  [ -n "$ASSET_URL" ] || error "No macOS asset found for $ARCH. If this release was just published, the macOS build may still be uploading — retry in a few minutes."
elif [ "$PLATFORM" = "linux" ]; then
  ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'x86_64-unknown-linux-gnu' || true)"
  [ -n "$ASSET_URL" ] || error "No Linux asset found for $ARCH. If this release was just published, the Linux build may still be uploading — retry in a few minutes."
fi

ASSET_NAME="$(basename "$ASSET_URL")"
info "Downloading ${ASSET_NAME}..."
# --speed-limit/--speed-time: abort if the transfer stalls (< 1 KB/s for 30s)
# instead of hanging silently on slow network routes.
curl -fL --progress-bar --retry 3 --retry-delay 2 \
  --speed-limit 1024 --speed-time 30 \
  "$ASSET_URL" -o "$TMP_DIR/$ASSET_NAME" \
  || error "Download failed or the connection stalled. Check your network and try again."

info "Extracting..."
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"
[ -x "$TMP_DIR/supacast" ] || error "Unexpected archive layout (no supacast binary found)."

install_binary() {
  local dest="$1"
  info "Installing to ${dest}..."
  if [ -w "$(dirname "$dest")" ] || mkdir -p "$(dirname "$dest")" 2>/dev/null; then
    mv "$TMP_DIR/supacast" "$dest" || error "Could not install to $dest."
  else
    sudo mv "$TMP_DIR/supacast" "$dest" || error "Could not install to $dest."
  fi
  chmod +x "$dest"
}

case "$PLATFORM" in
  darwin)
    # ---- Build Supacast.app: a real bundle so the app shows up in Finder
    # (/Applications), gets its rocket icon in Spotlight/Dock/notifications,
    # and macOS attributes permissions + notifications to the app itself.
    APP_DIR="${TMP_DIR}/Supacast.app"
    mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
    mv "$TMP_DIR/supacast" "$APP_DIR/Contents/MacOS/supacast"
    chmod +x "$APP_DIR/Contents/MacOS/supacast"
    printf 'APPL????' > "$APP_DIR/Contents/PkgInfo"

    cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>supacast</string>
    <key>CFBundleIdentifier</key><string>com.supacast.app</string>
    <key>CFBundleName</key><string>Supacast</string>
    <key>CFBundleDisplayName</key><string>Supacast</string>
    <key>CFBundleIconFile</key><string>Supacast</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleVersion</key><string>${VERSION:-1.0.0}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION:-1.0.0}</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSMicrophoneUsageDescription</key><string>Supacast uses the microphone for the dictate feature.</string>
    <key>NSHumanReadableCopyright</key><string>Copyright © 2026 harishdeivanayagam. All rights reserved.</string>
</dict>
</plist>
PLIST

    # Rocket icon (served from this site). Cosmetic — skip on failure.
    if curl -fsSL --speed-limit 1024 --speed-time 30 \
         "https://supacast-omega.vercel.app/Supacast.icns" \
         -o "$APP_DIR/Contents/Resources/Supacast.icns" 2>/dev/null; then
      info "Added rocket app icon."
    fi

    # Install into /Applications (admin users can write it directly; else sudo;
    # fall back to ~/Applications if the user declines sudo).
    if [ -w /Applications ] || sudo -n true 2>/dev/null; then
      if [ -w /Applications ]; then
        mv "$APP_DIR" /Applications/ || error "Could not install to /Applications."
      else
        sudo mv "$APP_DIR" /Applications/ || error "Could not install to /Applications."
      fi
      APP_DIR="/Applications/Supacast.app"
    else
      if sudo mv "$APP_DIR" /Applications/ 2>/dev/null; then
        APP_DIR="/Applications/Supacast.app"
      else
        mkdir -p "$HOME/Applications"
        mv "$APP_DIR" "$HOME/Applications/" || error "Could not install the app bundle."
        APP_DIR="$HOME/Applications/Supacast.app"
      fi
    fi

    # The binary is installed via curl, so Gatekeeper's "damaged" quarantine
    # stamp is never applied — but strip any xattrs anyway to be safe.
    xattr -cr "$APP_DIR" 2>/dev/null || true

    # Keep a `supacast` command on PATH (symlink into the bundle).
    if [ -w /usr/local/bin ] || [ -w /usr/local 2>/dev/null ]; then
      ln -sfn "$APP_DIR/Contents/MacOS/supacast" /usr/local/bin/supacast 2>/dev/null \
        || sudo ln -sfn "$APP_DIR/Contents/MacOS/supacast" /usr/local/bin/supacast
    else
      mkdir -p "$HOME/.local/bin"
      ln -sfn "$APP_DIR/Contents/MacOS/supacast" "$HOME/.local/bin/supacast"
      case ":$PATH:" in
        *":$HOME/.local/bin:"*) ;;
        *) info "Note: add $HOME/.local/bin to your PATH to run 'supacast'." ;;
      esac
    fi
    ;;
  linux)
    if [ -w /usr/local/bin ]; then
      install_binary "/usr/local/bin/supacast"
    else
      mkdir -p "$HOME/.local/bin"
      install_binary "$HOME/.local/bin/supacast"
      case ":$PATH:" in
        *":$HOME/.local/bin:"*) ;;
        *) info "Note: add $HOME/.local/bin to your PATH to run 'supacast'." ;;
      esac
    fi
    ;;
esac

printf '\n'
printf '\033[1;32m✔ Supacast installed successfully!\033[0m\n'
printf '\n'
case "$PLATFORM" in
  darwin)
    printf '  Supacast is installed in /Applications (drag to Trash to uninstall).\n'
    printf '  Starting it now — look for the menu-bar rocket icon.\n'
    open "$APP_DIR" 2>/dev/null || printf '  Run "supacast" or double-click Supacast.app to start it.\n'
    ;;
  linux)
    printf '  Run "supacast" to start it — look for the tray rocket icon.\n'
    printf '  Launch it once so it registers the global shortcut.\n'
    ;;
esac
printf '\n'
printf '  Docs: https://supacast-omega.vercel.app/docs/\n'
printf '  Made by https://github.com/harishdeivanayagam · https://zetahive.co\n'
