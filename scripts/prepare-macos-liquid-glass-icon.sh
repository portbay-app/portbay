#!/usr/bin/env bash
#
# prepare-macos-liquid-glass-icon.sh
# ----------------------------------
# DEV TOOL. Regenerates the committed macOS 26 (Tahoe) Liquid Glass asset
# catalog from the Icon Composer source. Run this when the icon art changes,
# then commit the regenerated Assets.car.
#
#   bash scripts/prepare-macos-liquid-glass-icon.sh   # then: git add compiled/Assets.car
#
# WHY THIS IS NO LONGER A BUILD STEP
#   The compiled catalog is committed at
#     src-tauri/icons/macos-liquid-glass/compiled/Assets.car
#   and wired statically into the bundle via tauri.conf.json:
#     bundle.resources["icons/macos-liquid-glass/compiled/Assets.car"] = "Assets.car"
#   plus src-tauri/Info.plist (CFBundleIconName=PortBay), which Tauri v2 merges
#   automatically. So EVERY build — local and CI, on any runner including
#   macos-14 without Xcode 26 — ships the Liquid Glass icon with zero build-time
#   work and without dirtying the working tree. This script only refreshes the
#   committed artifact; it does not touch tauri.conf.json or Info.plist.
#
# HOW THE CATALOG IS BUILT
#   actool takes the `.icon` document *itself* as a positional argument — NOT a
#   directory to scan (pointing it at a containing dir silently compiles
#   nothing). `--minimum-deployment-target 11.0` makes actool emit a legacy
#   `.icns` fallback inside the catalog alongside the macOS 26 layered icon, so
#   the one Assets.car serves every OS. Verified against Xcode 26.4.1.
#
# REQUIREMENTS
#   macOS 15+ with Xcode 26 selected (provides an `actool` that understands
#   standalone `.icon` documents). The default GitHub macos-14 runner cannot —
#   which is exactly why we commit the output instead of compiling in CI.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON_NAME="PortBay"
ICON_SRC="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/PortBay.icon"
OUT_DIR="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/compiled"
OUT_CAR="$OUT_DIR/Assets.car"

log()  { printf '\033[1;36m[icon]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[icon] WARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[icon] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "macOS only (needs Xcode 26 actool)."
command -v xcrun >/dev/null 2>&1 || die "xcrun not found — install Xcode 26."

xcode_major="$(xcodebuild -version 2>/dev/null | sed -n '1s/^Xcode \([0-9]*\).*/\1/p')"
if [ -z "$xcode_major" ] || [ "$xcode_major" -lt 26 ]; then
  die "Xcode ${xcode_major:-?} lacks .icon support (need 26+). Select an Xcode 26 toolchain with xcode-select."
fi
[ -d "$ICON_SRC" ] || die "Missing Icon Composer source: $ICON_SRC"

TMP_OUT="$(mktemp -d)"
trap 'rm -rf "$TMP_OUT"' EXIT

log "Compiling $ICON_NAME.icon with actool (Xcode $xcode_major)..."
# actool refuses to run unless the --compile output directory already exists.
mkdir -p "$TMP_OUT/out"
set +e
xcrun actool \
  "$ICON_SRC" \
  --app-icon "$ICON_NAME" \
  --compile "$TMP_OUT/out" \
  --output-partial-info-plist "$TMP_OUT/partial.plist" \
  --platform macosx \
  --minimum-deployment-target 11.0 \
  --target-device mac \
  --errors --warnings 2> "$TMP_OUT/actool.log"
rc=$?
set -e
if [ $rc -ne 0 ] || [ ! -f "$TMP_OUT/out/Assets.car" ]; then
  warn "actool log:"
  cat "$TMP_OUT/actool.log" >&2 || true
  die "actool failed (exit $rc); catalog not regenerated."
fi

mkdir -p "$OUT_DIR"
cp "$TMP_OUT/out/Assets.car" "$OUT_CAR"
log "Wrote $OUT_CAR"
log "Done. Commit the regenerated catalog:  git add ${OUT_CAR#$REPO_ROOT/}"
