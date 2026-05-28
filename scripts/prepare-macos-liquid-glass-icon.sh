#!/usr/bin/env bash
#
# prepare-macos-liquid-glass-icon.sh
# ----------------------------------
# PRE-BUILD step. Bakes the macOS 26 (Tahoe) Liquid Glass icon INTO the app
# bundle *before* `tauri build` signs and notarises it. Run this immediately
# before `pnpm tauri build` (the release workflow does).
#
# WHY PRE-BUILD AND NOT POST-BUILD?
#   `tauri build` compiles → signs → notarises → packages the .dmg and the
#   updater .app.tar.gz(+.sig) in a single shot. Injecting the icon AFTER that
#   would invalidate the notarisation staple and the updater signature, and
#   the already-packaged .dmg/tarball would still hold the un-injected .app.
#   So for release artifacts the catalog must be present *before* Tauri signs.
#   (scripts/inject-macos-liquid-glass-icon.sh is the POST-build counterpart —
#   for quick LOCAL preview only, where ad-hoc re-signing is fine.)
#
# HOW IT BAKES IN (no re-sign needed):
#   - Stages a compiled `Assets.car` and registers it as a Tauri bundle
#     resource, so Tauri copies it to PortBay.app/Contents/Resources/Assets.car
#     during assembly (before signing).
#   - Writes `CFBundleIconName` into src-tauri/Info.plist, which Tauri v2
#     merges into the generated Info.plist. The bundled icon.icns
#     (CFBundleIconFile) is left intact as the macOS 11–15 fallback.
#
#   These edits land in the CI workspace (a fresh checkout) and the staging
#   dir is git-ignored, so the committed config is never mutated.
#
# NON-FATAL BY DESIGN:
#   If no asset catalog can be produced (no committed compiled/Assets.car AND
#   no Xcode 26 actool on the runner), this prints a warning and exits 0 — the
#   build proceeds and ships the .icns fallback. It only fails hard if a
#   catalog IS available but staging it fails.
#
# REQUIREMENTS for the glass icon to actually ship via CI (either one):
#   - Commit a pre-compiled catalog at
#     src-tauri/icons/macos-liquid-glass/compiled/Assets.car  (works on any
#     runner, including macos-14), OR
#   - Run on a macOS 15+ runner with Xcode 26 selected (provides an `actool`
#     that understands .icon documents). The default macos-14 runner cannot.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON_NAME="PortBay"
ICON_SRC="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/PortBay.icon"
PREBUILT_CAR="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/compiled/Assets.car"
STAGE_DIR="$REPO_ROOT/src-tauri/icons/macos-liquid-glass/.staged"
STAGED_CAR="$STAGE_DIR/Assets.car"
STAGED_CAR_REL="icons/macos-liquid-glass/.staged/Assets.car"  # relative to src-tauri (tauri.conf dir)
CONF="$REPO_ROOT/src-tauri/tauri.conf.json"
INFO_PLIST="$REPO_ROOT/src-tauri/Info.plist"

log()  { printf '\033[1;36m[icon]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[icon] WARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[icon] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# Non-macOS CI (Linux/Windows jobs) has nothing to do here.
if [ "$(uname -s)" != "Darwin" ]; then
  log "Non-macOS host — nothing to bake. Skipping."
  exit 0
fi

# --- resolve a compiled Assets.car ----------------------------------------
TMP_OUT="$(mktemp -d)"
trap 'rm -rf "$TMP_OUT"' EXIT
CAR_SRC=""

if [ -f "$PREBUILT_CAR" ]; then
  log "Using committed catalog: $PREBUILT_CAR"
  CAR_SRC="$PREBUILT_CAR"
elif command -v xcrun >/dev/null 2>&1; then
  # Xcode 26 ships an actool that understands standalone .icon documents.
  # Probe the Xcode version; skip gracefully if it's too old (e.g. macos-14).
  xcode_major="$(xcodebuild -version 2>/dev/null | sed -n '1s/^Xcode \([0-9]*\).*/\1/p')"
  if [ -z "$xcode_major" ] || [ "$xcode_major" -lt 26 ]; then
    warn "Xcode ${xcode_major:-?} lacks .icon support (need 26+). No committed compiled/Assets.car either."
    warn "Shipping the icon.icns fallback. To ship the Liquid Glass icon, commit compiled/Assets.car or use an Xcode 26 runner."
    exit 0
  fi
  [ -d "$ICON_SRC" ] || die "Missing Icon Composer source: $ICON_SRC"
  log "Compiling $ICON_NAME.icon with actool (Xcode $xcode_major)..."
  # actool takes the `.icon` document *itself* as a positional argument — NOT a
  # directory to scan (pointing it at a containing dir silently compiles
  # nothing). `--minimum-deployment-target 11.0` makes actool emit a legacy
  # `.icns` fallback inside the catalog alongside the macOS 26 layered icon, so
  # the one Assets.car serves every OS. Verified against Xcode 26.4.1.
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
    warn "actool failed (exit $rc); shipping .icns fallback. Log:"
    cat "$TMP_OUT/actool.log" >&2 || true
    exit 0
  fi
  CAR_SRC="$TMP_OUT/out/Assets.car"
else
  warn "No committed compiled/Assets.car and no xcrun available. Shipping .icns fallback."
  exit 0
fi

# --- stage the catalog as a Tauri bundle resource -------------------------
# Past this point a catalog IS available, so failures are real errors.
mkdir -p "$STAGE_DIR"
cp "$CAR_SRC" "$STAGED_CAR"
log "Staged catalog at $STAGED_CAR"

log "Registering bundle resource ($STAGED_CAR_REL -> Resources/Assets.car) in tauri.conf.json"
node - "$CONF" "$STAGED_CAR_REL" <<'NODE'
const fs = require('fs');
const [confPath, carRel] = process.argv.slice(2);
const conf = JSON.parse(fs.readFileSync(confPath, 'utf8'));
conf.bundle = conf.bundle || {};
// Resource map: source (relative to src-tauri) -> dest (relative to the
// platform resource dir; on macOS that is Contents/Resources/).
const res = conf.bundle.resources;
if (Array.isArray(res)) {
  // Convert an array form to the map form so we can target an exact dest.
  const map = {};
  for (const p of res) map[p] = p.split('/').pop();
  conf.bundle.resources = map;
} else if (!res || typeof res !== 'object') {
  conf.bundle.resources = {};
}
conf.bundle.resources[carRel] = 'Assets.car';
fs.writeFileSync(confPath, JSON.stringify(conf, null, 2) + '\n');
console.log('  bundle.resources now:', JSON.stringify(conf.bundle.resources));
NODE

log "Writing CFBundleIconName=$ICON_NAME into src-tauri/Info.plist (Tauri merges this)"
if [ ! -f "$INFO_PLIST" ]; then
  cat > "$INFO_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIconName</key>
	<string>$ICON_NAME</string>
</dict>
</plist>
PLIST
else
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconName string $ICON_NAME" "$INFO_PLIST" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c "Set :CFBundleIconName $ICON_NAME" "$INFO_PLIST"
fi

log "Baked. The following 'tauri build' will include Assets.car and sign/notarise the bundle with the Liquid Glass icon."
