#!/bin/sh
# Regenerates assets/AppIcon.icns from scripts/icon_art.rs.
#
# You only need this if you are CHANGING the icon. The .icns is committed, so
# building and installing the app never runs this.
#
# The renderer is a standalone rustc file rather than a cargo target, so it
# needs no dependency and no crate restructuring, and the only toolchain
# required is the one that already builds the app.
set -eu

cd "$(dirname "$0")/.."
OUT="assets/AppIcon.icns"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v iconutil >/dev/null 2>&1 || {
  echo "iconutil not found. It ships with macOS; this script only runs there." >&2
  exit 1
}

rustc -O scripts/icon.rs -o "$TMP/icon"
"$TMP/icon" "$TMP/AppIcon.iconset"

mkdir -p assets
iconutil -c icns "$TMP/AppIcon.iconset" -o "$OUT"

# Windows has no bundle to carry an .icns, so the icon is compiled into the
# binary there. Generated from the same geometry in the same run, so the two
# cannot drift apart.
"$TMP/icon" --rgba 128 assets/icon-128.rgba >/dev/null

echo
echo "wrote $OUT ($(wc -c < "$OUT" | tr -d ' ') bytes)"
echo "wrote assets/icon-128.rgba ($(wc -c < assets/icon-128.rgba | tr -d ' ') bytes, for Windows)"
echo "run scripts/make-app-bundle.sh to put it into the app"
