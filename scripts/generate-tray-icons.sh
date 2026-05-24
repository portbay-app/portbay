#!/usr/bin/env bash
# Generate the four status-aware tray icons used by the macOS menu bar.
#
# Output: src-tauri/icons/tray/{idle,starting,running,error}.png at 44x44
# (rendered as ~22pt in the macOS menu bar — crisp on Retina, downscales
# cleanly on non-Retina). Glyph: a hollow circle with a centred dot — the
# universally-readable "port" symbol — tinted by status colour.
#
# Requires ImageMagick. Re-run after editing colours; checked-in PNGs are
# the source of truth at runtime.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/src-tauri/icons/tray"
mkdir -p "$OUT"

# Canvas: 44x44 transparent, content centred. The status taxonomy here is
# the same vocabulary the projects table uses — keep them in sync.
SIZE=44
RING=3   # outer ring stroke

render() {
  local name="$1" colour="$2"
  magick -size ${SIZE}x${SIZE} xc:none \
    -fill none -stroke "$colour" -strokewidth "$RING" \
    -draw "circle 22,22 22,4" \
    -fill "$colour" -stroke none \
    -draw "circle 22,22 22,15" \
    "$OUT/$name.png"
}

# Gray — daemon not running / no projects healthy
render idle     "#9CA3AF"
# Blue — at least one project starting; nothing crashed
render starting "#3B82F6"
# Green — all running projects healthy
render running  "#22C55E"
# Red — at least one project crashed or port-conflicted
render error    "#EF4444"

echo "Wrote tray icons to $OUT"
