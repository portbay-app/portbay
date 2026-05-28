#!/usr/bin/env bash
#
# inject-macos-liquid-glass-icon.sh
# ---------------------------------
# Post-build step that gives the built PortBay.app the macOS 26 (Tahoe)
# "Liquid Glass" app icon authored in Apple Icon Composer.
#
# WHY THIS EXISTS
#   Tauri's bundler can only embed a single static `icon.icns` (see the
#   `bundle.icon` array in tauri.conf.json). It has no concept of an
#   Icon Composer `.icon` document, and it does not emit a compiled
#   asset catalog (`Assets.car`) or set `CFBundleIconName`. The macOS 26
#   icon system — the rim, specular shine, translucency, and the
#   Default / Dark / Clear-Light / Clear-Dark / Tinted appearances — is
#   rendered by the OS from a compiled asset catalog, NOT from an .icns.
#
#   So we keep Tauri's `.icns` as the pre-Tahoe fallback and, after the
#   bundle is built, compile the `.icon` into an `Assets.car`, drop it
#   into the .app, and add `CFBundleIconName` to Info.plist. macOS 26
#   prefers `CFBundleIconName`/Assets.car and falls back to the `.icns`
#   on macOS 11–15.
#
#   None of the glass treatment is synthesized here. We only compile and
#   reference Apple's own asset format. The look is 100% the system's.
#
# USAGE
#   bash scripts/inject-macos-liquid-glass-icon.sh [path/to/PortBay.app]
#
#   With no argument it auto-discovers the most recently built bundle under
#   src-tauri/target/**/bundle/macos/PortBay.app.
#
# REQUIREMENTS
#   - macOS host with Xcode 26+ (provides an `actool` that understands
#     `.icon` documents). Command Line Tools alone are NOT enough.
#   - Alternatively, a pre-compiled Assets.car committed at
#     src-tauri/icons/macos-liquid-glass/compiled/Assets.car (see README).
#
# ENV
#   APPLE_SIGNING_IDENTITY  If set, the patched bundle is re-signed with it.
#                           If unset, an ad-hoc signature is applied and a
#                           warning is printed (fine for local testing only).
#
set -euo pipefail

# --- locations -------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON_SRC="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/PortBay.icon"
PREBUILT_CAR="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/compiled/Assets.car"
ICON_NAME="PortBay"   # basename of the .icon -> value used for CFBundleIconName

log()  { printf '\033[1;36m[icon]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[icon] WARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[icon] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- platform guard --------------------------------------------------------
[ "$(uname -s)" = "Darwin" ] || die "macOS-only step. This is a no-op on Windows/Linux; skip it there."

# --- resolve the .app ------------------------------------------------------
APP_PATH="${1:-}"
if [ -z "$APP_PATH" ]; then
  APP_PATH="$(find "$REPO_ROOT/src-tauri/target" -type d -name 'PortBay.app' -path '*/bundle/macos/*' 2>/dev/null \
    | xargs -I {} stat -f '%m %N' {} 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)"
fi
[ -n "$APP_PATH" ] && [ -d "$APP_PATH" ] || die "Could not find PortBay.app. Build first (pnpm tauri build) or pass the path explicitly."
log "Target bundle: $APP_PATH"

RESOURCES="$APP_PATH/Contents/Resources"
INFO_PLIST="$APP_PATH/Contents/Info.plist"
[ -d "$RESOURCES" ] || die "Malformed bundle: $RESOURCES missing"
[ -f "$INFO_PLIST" ] || die "Malformed bundle: $INFO_PLIST missing"

# --- obtain a compiled Assets.car -----------------------------------------
# Preference order:
#   1. A pre-compiled Assets.car committed to the repo (deterministic; works
#      on CI without Xcode 26). See compiled/README in macos-liquid-glass.
#   2. Compile the .icon on the fly with `actool` from Xcode 26.
TMP_OUT="$(mktemp -d)"
trap 'rm -rf "$TMP_OUT"' EXIT

CAR_SRC=""
if [ -f "$PREBUILT_CAR" ]; then
  log "Using pre-compiled asset catalog: $PREBUILT_CAR"
  CAR_SRC="$PREBUILT_CAR"
else
  [ -d "$ICON_SRC" ] || die "Missing Icon Composer source: $ICON_SRC"
  command -v xcrun >/dev/null 2>&1 || die "xcrun not found. Install Xcode 26+."

  log "Compiling $ICON_NAME.icon with actool (requires Xcode 26+)..."
  # actool takes the `.icon` document *itself* as a positional argument — NOT a
  # directory to scan (pointing it at a containing dir silently compiles
  # nothing). The `--compile` output dir must already exist. And
  # `--minimum-deployment-target 11.0` makes actool emit a legacy `.icns`
  # fallback inside the catalog alongside the macOS 26 layered icon, so the one
  # Assets.car serves every OS. Verified against Xcode 26.4.1.
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
    warn "actool did not produce Assets.car (exit $rc). Log:"
    cat "$TMP_OUT/actool.log" >&2 || true
    die "Compile the .icon in Xcode 26 and commit compiled/Assets.car, then re-run. See src-tauri/icons/macos-liquid-glass/README.md."
  fi
  CAR_SRC="$TMP_OUT/out/Assets.car"
fi

# --- inject ----------------------------------------------------------------
log "Copying Assets.car into bundle Resources"
cp "$CAR_SRC" "$RESOURCES/Assets.car"

log "Setting CFBundleIconName=$ICON_NAME in Info.plist"
# Add or overwrite the key. PlistBuddy 'Set' fails if absent, so try Add first.
/usr/libexec/PlistBuddy -c "Add :CFBundleIconName string $ICON_NAME" "$INFO_PLIST" 2>/dev/null \
  || /usr/libexec/PlistBuddy -c "Set :CFBundleIconName $ICON_NAME" "$INFO_PLIST"

# Keep CFBundleIconFile (the .icns) intact as the macOS 11–15 fallback — do
# not remove it. Both keys coexist; newer systems prefer CFBundleIconName.

# --- re-sign (icon injection invalidates the signature) --------------------
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  log "Re-signing with APPLE_SIGNING_IDENTITY"
  codesign --force --deep --options runtime \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP_PATH"
else
  warn "APPLE_SIGNING_IDENTITY unset — applying ad-hoc signature (local testing only; not distributable)."
  codesign --force --deep --sign - "$APP_PATH"
fi

log "Done. Verify with: codesign --verify --deep --strict --verbose=2 \"$APP_PATH\""
log "Inspect icon name with: /usr/libexec/PlistBuddy -c 'Print :CFBundleIconName' \"$INFO_PLIST\""
