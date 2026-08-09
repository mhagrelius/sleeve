#!/usr/bin/env bash
#
# Install Sleeve into the user's home directory. No root, no system paths —
# everything lands under ~/.local.
#
#   ./install.sh
#   PREFIX=/usr/local sudo ./install.sh
#
set -euo pipefail

APP_ID="us.hagreli.Sleeve"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
DATA_DIR="$PREFIX/share"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m warning:\033[0m %s\n' "$*" >&2; }

say "Building (release)"
cargo build --release --locked

say "Installing to $PREFIX"
install -Dm755 target/release/sleeve "$BIN_DIR/sleeve"
install -Dm644 "data/$APP_ID.desktop" "$DATA_DIR/applications/$APP_ID.desktop"
install -Dm644 "data/$APP_ID.metainfo.xml" "$DATA_DIR/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "data/icons/hicolor/scalable/apps/$APP_ID.svg" \
  "$DATA_DIR/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
  "$DATA_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

if command -v gtk4-update-icon-cache >/dev/null; then
  gtk4-update-icon-cache -qtf "$DATA_DIR/icons/hicolor" 2>/dev/null || true
elif command -v gtk-update-icon-cache >/dev/null; then
  gtk-update-icon-cache -qtf "$DATA_DIR/icons/hicolor" 2>/dev/null || true
fi
if command -v update-desktop-database >/dev/null; then
  update-desktop-database -q "$DATA_DIR/applications" 2>/dev/null || true
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "$BIN_DIR is not on your PATH; add it to run 'sleeve' from a terminal" ;;
esac

config="${XDG_CONFIG_HOME:-$HOME/.config}/sleeve/config.toml"
echo
say "Installed. Run it with: sleeve"
echo
say "No accounts, no API keys, nothing to sign up for — every source works as is."
echo
say "Sleeve writes an annotated config on first run:"
say "  $config"
say "Worth setting there: locale and currency, which decide the storefront asked"
say "and therefore the prices; and contact, so MusicBrainz reaches you rather than"
say "blocking the project URL. The API keys are optional and only add detail."
