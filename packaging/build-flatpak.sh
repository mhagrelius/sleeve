#!/usr/bin/env bash
#
# Build and install the Sleeve Flatpak.
#
#   packaging/build-flatpak.sh            build and install --user
#   packaging/build-flatpak.sh --bundle   also write dist/sleeve.flatpak
#
set -euo pipefail

APP_ID="us.hagreli.Sleeve"
RUNTIME_VERSION="50"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

MANIFEST="packaging/flatpak/$APP_ID.yml"
SOURCES="packaging/flatpak/cargo-sources.json"
BUILD_DIR="$here/.flatpak-build"
DIST="$here/dist"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

command -v flatpak >/dev/null || die "flatpak is not installed"
command -v flatpak-builder >/dev/null \
  || die "flatpak-builder is not installed (sudo apt install flatpak-builder)"

# The rust-stable extension is versioned by the *freedesktop* base the GNOME SDK
# is built on, not by the GNOME version. Read it from the SDK rather than
# hardcoding a number that silently rots into a failed build one release later.
BASE_VERSION="$(flatpak remote-info --show-metadata flathub "org.gnome.Sdk//$RUNTIME_VERSION" 2>/dev/null \
  | sed -n 's/^runtime=org.freedesktop.Platform\/[^/]*\/\(.*\)$/\1/p' | head -1)"
BASE_VERSION="${BASE_VERSION:-25.08}"

say "Installing runtimes"
flatpak install --user --noninteractive --or-update flathub \
  "org.gnome.Platform//$RUNTIME_VERSION" \
  "org.gnome.Sdk//$RUNTIME_VERSION" \
  "org.freedesktop.Sdk.Extension.rust-stable//$BASE_VERSION"

# Every crate, pinned by checksum, so the build needs no network.
say "Generating $SOURCES from Cargo.lock"
GENERATOR="$BUILD_DIR/flatpak-cargo-generator.py"
mkdir -p "$BUILD_DIR"
if [[ ! -f "$GENERATOR" ]]; then
  curl -sSfL -o "$GENERATOR" \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py \
    || die "could not fetch flatpak-cargo-generator.py"
fi
python3 "$GENERATOR" Cargo.lock -o "$SOURCES"

say "Building"
flatpak-builder --user --force-clean --install \
  --state-dir "$BUILD_DIR/state" \
  "$BUILD_DIR/repo" "$MANIFEST"

if [[ "${1:-}" == "--bundle" ]]; then
  mkdir -p "$DIST"
  say "Writing $DIST/sleeve.flatpak"
  flatpak build-bundle "$BUILD_DIR/state/repo" "$DIST/sleeve.flatpak" "$APP_ID" \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo
fi

echo
say "Installed. Run with: flatpak run $APP_ID"
