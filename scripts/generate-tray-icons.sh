#!/usr/bin/env bash
# Generate the macOS menu-bar tray icon from the PortBay mark.
#
# Output: src-tauri/icons/tray/icon-template.png at 44x44 (rendered as
# ~22pt in the menu bar — crisp on Retina, downscales cleanly on
# non-Retina). The PNG is a pure-black silhouette on transparent so it
# can be flagged as a macOS *template image* (`icon_as_template(true)` in
# tray.rs): the OS tints it automatically — black on a light menu bar,
# white on a dark one — giving native appearance switching for free.
#
# Source of truth: src-tauri/icons/tray/icon-template.svg. Re-run this
# after editing the SVG; the checked-in PNG is what's embedded at runtime.
#
# Requires ImageMagick.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/src-tauri/icons/tray"
SRC="$OUT/icon-template.svg"
mkdir -p "$OUT"

# Canvas 44x44, glyph fit to 36px and centred → ~4px breathing room so the
# mark isn't edge-to-edge in the menu bar. `-evaluate set 0` on the RGB
# channels forces the rasterised shape to pure black while preserving the
# anti-aliased alpha — the contract a template image expects.
magick -background none "$SRC" \
  -resize 36x36 \
  -channel RGB -evaluate set 0 +channel \
  -gravity center -background none -extent 44x44 \
  "$OUT/icon-template.png"

echo "Wrote tray template icon to $OUT/icon-template.png"
