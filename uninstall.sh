#!/usr/bin/env bash
#
# Remove what install.sh installed.
#
set -euo pipefail

APP_ID="us.hagreli.Sleeve"
PREFIX="${PREFIX:-$HOME/.local}"
DATA_DIR="$PREFIX/share"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

say "Removing Sleeve from $PREFIX"
rm -f "$PREFIX/bin/sleeve"
rm -f "$DATA_DIR/applications/$APP_ID.desktop"
rm -f "$DATA_DIR/metainfo/$APP_ID.metainfo.xml"
rm -f "$DATA_DIR/icons/hicolor/scalable/apps/$APP_ID.svg"
rm -f "$DATA_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

if command -v update-desktop-database >/dev/null; then
  update-desktop-database -q "$DATA_DIR/applications" 2>/dev/null || true
fi

echo
say "Done. These were left alone, because one of them has your API tokens in it:"
say "  ~/.config/sleeve   config.toml"
say "  ~/.cache/sleeve    cached responses and cover art"
echo
say "  rm -r ~/.config/sleeve ~/.cache/sleeve"
