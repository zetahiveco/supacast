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

ASSET_URL=""
if [ "$PLATFORM" = "darwin" ]; then
  if [ "$ARCH" = "arm64" ]; then
    ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'aarch64.*\.app\.tar\.gz' || true)"
    [ -n "$ASSET_URL" ] || ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'aarch64.*\.dmg' || true)"
  else
    ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E '(x64|x86_64).*\.app\.tar\.gz' || true)"
    [ -n "$ASSET_URL" ] || ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E '(x64|x86_64).*\.dmg' || true)"
  fi
  [ -n "$ASSET_URL" ] || error "No macOS asset found for $ARCH."
elif [ "$PLATFORM" = "linux" ]; then
  ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'amd64\.deb' || true)"
  [ -n "$ASSET_URL" ] || ASSET_URL="$(printf '%s\n' "$ASSET_URLS" | grep -m1 -E 'amd64.*\.AppImage' || true)"
  [ -n "$ASSET_URL" ] || error "No Linux asset found for $ARCH."
fi

ASSET_NAME="$(basename "$ASSET_URL")"
info "Downloading ${ASSET_NAME}..."
curl -fL "$ASSET_URL" -o "$TMP_DIR/$ASSET_NAME" \
  || error "Download failed."

install_macos() {
  local archive="$TMP_DIR/$ASSET_NAME"
  # Installed via curl, so Gatekeeper's "damaged" quarantine stamp is never
  # applied — but strip any xattrs anyway to guarantee the app launches.
  local target="/Applications/${APP_NAME}.app"
  if [[ "$ASSET_NAME" == *.app.tar.gz ]]; then
    tar -xzf "$archive" -C "$TMP_DIR"
    [ -d "$TMP_DIR/${APP_NAME}.app" ] || error "Unexpected archive layout (no ${APP_NAME}.app found)."
    info "Installing to /Applications..."
    rm -rf "$target"
    mv "$TMP_DIR/${APP_NAME}.app" /Applications/ 2>/dev/null \
      || sudo mv "$TMP_DIR/${APP_NAME}.app" /Applications/ \
      || error "Could not write to /Applications."
  else
    # .dmg fallback: mount, copy, unmount
    info "Mounting disk image..."
    hdiutil attach "$archive" -nobrowse -quiet || error "Could not mount dmg."
    MOUNT_DIR="/Volumes/${APP_NAME}"
    info "Installing to /Applications..."
    rm -rf "$target"
    cp -R "$MOUNT_DIR/${APP_NAME}.app" /Applications/ 2>/dev/null \
      || sudo cp -R "$MOUNT_DIR/${APP_NAME}.app" /Applications/ \
      || { hdiutil detach "$MOUNT_DIR" -quiet || true; error "Could not write to /Applications."; }
    hdiutil detach "$MOUNT_DIR" -quiet || true
  fi
  xattr -cr "$target" 2>/dev/null || true
  info "Done — launching is not blocked (no quarantine applied)."
}

install_linux() {
  if [[ "$ASSET_NAME" == *.deb ]]; then
    info "Installing deb package..."
    sudo dpkg -i "$TMP_DIR/$ASSET_NAME" || sudo apt-get install -f -y \
      || error "dpkg install failed."
  else
    local dest="${HOME}/.local/bin"
    mkdir -p "$dest"
    mv "$TMP_DIR/$ASSET_NAME" "$dest/${APP_NAME}.AppImage"
    chmod +x "$dest/${APP_NAME}.AppImage"
    info "Installed AppImage to $dest/${APP_NAME}.AppImage"
  fi
}

info "Installing..."
case "$PLATFORM" in
  darwin) install_macos ;;
  linux)  install_linux ;;
esac

printf '\n'
printf '\033[1;32m✔ Supacast installed successfully!\033[0m\n'
printf '\n'
case "$PLATFORM" in
  darwin)
    printf '  Open it from /Applications or the menu-bar rocket icon.\n'
    printf '  It will now start automatically every time you log in.\n'
    ;;
  linux)
    printf '  Launch it from your application menu (or the AppImage path above).\n'
    ;;
esac
printf '\n'
printf '  Docs: https://supacast-omega.vercel.app/docs/\n'
printf '  Made by https://github.com/harishdeivanayagam · https://zetahive.co\n'