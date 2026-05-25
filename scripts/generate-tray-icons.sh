#!/usr/bin/env bash
# Generate the four tray icons used by the macOS menu bar.
#
# All icons are black-on-transparent monochrome so they work as macOS
# template images (icon_as_template = true). macOS auto-inverts them for
# dark menu bars and applies vibrancy — no manual light/dark variants needed.
#
# Output: src-tauri/icons/tray/{idle,starting,running,error}.png at 44x44
# (rendered as ~22pt in the menu bar). Glyph: hollow ring + centre dot.
#
# Requires ImageMagick. Re-run after editing shapes; checked-in PNGs are
# the source of truth at runtime.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/src-tauri/icons/tray"
mkdir -p "$OUT"

SIZE=44
RING=3

render() {
  local name="$1"
  magick -size ${SIZE}x${SIZE} xc:none \
    -fill none -stroke black -strokewidth "$RING" \
    -draw "circle 22,22 22,4" \
    -fill black -stroke none \
    -draw "circle 22,22 22,15" \
    "$OUT/$name.png"
}

render idle
render starting
render running
render error

echo "Wrote monochrome tray icons to $OUT"
