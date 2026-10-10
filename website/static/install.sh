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
    # The binary is installed via curl, so Gatekeeper's "damaged" quarantine
    # stamp is never applied — but strip any xattrs anyway to be safe.
    xattr -cr "$HOME/.local/bin/supacast" 2>/dev/null || true
    xattr -cr /usr/local/bin/supacast 2>/dev/null || true
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
    printf '  Run "supacast" to start it — look for the menu-bar rocket icon.\n'
    printf '  Launch it once so it registers the global shortcut.\n'
    ;;
  linux)
    printf '  Run "supacast" to start it — look for the tray rocket icon.\n'
    printf '  Launch it once so it registers the global shortcut.\n'
    ;;
esac
printf '\n'
printf '  Docs: https://supacast-omega.vercel.app/docs/\n'
printf '  Made by https://github.com/harishdeivanayagam · https://zetahive.co\n'
