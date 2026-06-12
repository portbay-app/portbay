#!/usr/bin/env bash
#
# Build a BRANDED + signed + notarized + stapled PortBay DMG, entirely locally.
#
# Why local: the styled DMG window (background, PortBay logo, drag-to-Applications
# layout) is applied by Finder via AppleScript. GitHub's headless CI runners can't
# drive Finder (error -1743 "Operation not permitted"), so CI ships a plain-folder
# DMG. Running here on your Mac, Finder is available and the branded window works.
#
# Notarization is done explicitly with `notarytool --wait --progress` so you SEE
# live status instead of the opaque hang you get when `tauri build` notarizes
# silently in the background.
#
# Usage:
#   1. Fill in the 4 values in the CONFIG block below.
#   2. bash scripts/release-dmg-local.sh
#
set -euo pipefail

# ============================ CONFIG ============================
# The ONLY value you must supply: your App Store Connect Issuer ID (a UUID,
# App Store Connect > Users and Access > Integrations > Keys > "Issuer ID").
# Pass it inline:  ISSUER=xxxxxxxx-... bash scripts/release-dmg-local.sh
# or paste it here between the quotes:
ISSUER="${ISSUER:-}"

# Everything below is auto-resolved from files already on this machine:
#   .secrets/portbay-updater.key            -> Tauri updater key
#   .secrets/portbay-updater.key.password   -> its password
#   ~/.appstoreconnect/private_keys/AuthKey_LLRT66A2TF.p8 -> notary key (Key ID LLRT66A2TF)
UPDATER_KEY_FILE="${UPDATER_KEY_FILE:-.secrets/portbay-updater.key}"
UPDATER_KEY_PASSWORD_FILE="${UPDATER_KEY_PASSWORD_FILE:-.secrets/portbay-updater.key.password}"
P8_PATH="${P8_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_LLRT66A2TF.p8}"
KEY_ID="${KEY_ID:-LLRT66A2TF}"
# ===============================================================

SIGNING_IDENTITY="Developer ID Application: Tribal House LLC (V2CYH6HZT8)"
TARGET="aarch64-apple-darwin"
# Self-relative (the pattern the other scripts use) — no machine-local path.
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$REPO_DIR"

# ---- Preflight: fail early with a clear message instead of a confusing build error
die() { echo "ERROR: $*" >&2; exit 1; }

[[ -n "$ISSUER" ]]      || die "Set your App Store Connect Issuer ID (see CONFIG block at top)."
[[ -f "$P8_PATH" ]]     || die "App Store Connect key not found at: $P8_PATH"
[[ -f "$UPDATER_KEY_FILE" ]] || die "Updater key not found at: $UPDATER_KEY_FILE"
security find-identity -v -p codesigning | grep -q "$SIGNING_IDENTITY" \
  || die "Signing identity not in keychain: $SIGNING_IDENTITY"

echo "==> Signing identity, notary key, and updater key all present."

# ---- Signing env for `tauri build` (read key contents from the .secrets files).
export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$UPDATER_KEY_FILE")"
# Exporting this (even empty) stops Tauri from prompting on stdin for a key
# password — an interactive prompt is what makes the build look "stuck".
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$([[ -f "$UPDATER_KEY_PASSWORD_FILE" ]] && cat "$UPDATER_KEY_PASSWORD_FILE" || true)"

# CRITICAL: keep notary creds OUT of the build env so Tauri does NOT notarize
# silently (that is the 30-min black box). We notarize ourselves below, visibly.
unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH

# ---- Sidecar prep (mirrors CI so the bundle is complete and current).
echo "==> Preparing sidecar binaries..."
./scripts/fetch-process-compose.sh
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
./scripts/fetch-dnsmasq.sh
TARGET_TRIPLE="$TARGET" ./scripts/build-hosts-helper.sh
TARGET_TRIPLE="$TARGET" ./scripts/build-mcp.sh
TARGET_TRIPLE="$TARGET" ./scripts/build-afm.sh
# PortBay agent sidecar (Cline-derived fork TUI): rebuild only when the
# vendored fork checkout is reachable (private dev machines). A checkout
# without it bundles whatever src-tauri/binaries/ already holds — same
# posture as the other locally-built AI sidecars. Tauri then signs it with
# Entitlements.plist (the bun binary needs its allow-jit under hardened
# runtime, or it dies on launch in the notarized bundle).
agent_fork_dir="${PORTBAY_CLINE_DIR:-${REPO_DIR}/../portbay-cline}"
if [ -f "${agent_fork_dir}/apps/cli/package.json" ]; then
  # build-portbay-agent.sh runs the check-agent-branding ship gate itself.
  TARGET_TRIPLE="$TARGET" PORTBAY_CLINE_DIR="$agent_fork_dir" ./scripts/build-portbay-agent.sh
else
  echo "==> portbay-cline fork not found at ${agent_fork_dir} — bundling existing portbay-agent binary as-is."
  ./scripts/check-agent-branding.sh "src-tauri/binaries/portbay-agent-${TARGET}"
fi

# ---- macOS 26 Liquid Glass icon is wired statically via bundle.resources +
# src-tauri/Info.plist (compiled/Assets.car is committed), so a plain build
# already includes it — no pre-build bake step needed.

# ---- Build: signs the app + sidecars, and styles the DMG via Finder (local GUI).
# Enable the proprietary task board when the desktop-pro overlay is present (same
# `tasks` feature scripts/dev-pro.sh uses for `tauri dev`, and that build-mcp.sh
# auto-detects for the sidecar above). A public OSS checkout lacks src/context,
# so the DMG stays board-free with no flag.
echo "==> Building + signing (Finder will style the DMG)..."
pnpm install --frozen-lockfile
tauri_feature_args=()
if [ -f "$REPO_DIR/src-tauri/src/context/board.rs" ]; then
  tauri_feature_args=(--features tasks)
  echo "==> desktop-pro overlay detected — building app with --features tasks"
fi
pnpm tauri build --target "$TARGET" --bundles app,dmg "${tauri_feature_args[@]}"

DMG="$(ls -t "src-tauri/target/$TARGET/release/bundle/dmg/"*.dmg | head -1)"
[[ -n "$DMG" && -f "$DMG" ]] || die "No DMG was produced."
echo "==> Built styled DMG: $DMG"

# ---- Notarize the DMG itself (live progress; ~5-15 min on Apple's side).
echo "==> Submitting DMG to Apple notary service..."
xcrun notarytool submit "$DMG" \
  --key "$P8_PATH" \
  --key-id "$KEY_ID" \
  --issuer "$ISSUER" \
  --wait --progress

# ---- Staple the ticket so it verifies offline / at mount.
echo "==> Stapling..."
xcrun stapler staple "$DMG"

# ---- Verify the wrapper passes Gatekeeper.
echo "==> Verifying..."
xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -vv "$DMG"

# ---- Version-name the updater artifacts. Tauri emits the updater tarball as
# an unversioned `PortBay.app.tar.gz`; every asset a user downloads must carry
# the version in its filename (v0.1.4 shipped a versionless tarball). The DMG
# is already versioned by the bundler (`PortBay_X.Y.Z_aarch64.dmg`).
VERSION="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' src-tauri/tauri.conf.json | head -1)"
[[ -n "$VERSION" ]] || die "Couldn't read version from src-tauri/tauri.conf.json"
MACOS_BUNDLE_DIR="src-tauri/target/$TARGET/release/bundle/macos"
TARBALL="$(ls -t "$MACOS_BUNDLE_DIR/"*.app.tar.gz 2>/dev/null | head -1 || true)"
VERSIONED_TARBALL=""
if [[ -n "$TARBALL" && "$(basename "$TARBALL")" != *"$VERSION"* ]]; then
  VERSIONED_TARBALL="$MACOS_BUNDLE_DIR/PortBay_${VERSION}_aarch64.app.tar.gz"
  cp "$TARBALL" "$VERSIONED_TARBALL"
  [[ -f "$TARBALL.sig" ]] && cp "$TARBALL.sig" "$VERSIONED_TARBALL.sig"
elif [[ -n "$TARBALL" ]]; then
  VERSIONED_TARBALL="$TARBALL"
fi

echo ""
echo "DONE. Branded, signed, notarized, stapled DMG:"
echo "  $DMG"
if [[ -n "$VERSIONED_TARBALL" ]]; then
  echo "Updater tarball (versioned — upload THIS name and reference it in latest.json):"
  echo "  $VERSIONED_TARBALL"
  echo "  $VERSIONED_TARBALL.sig"
fi
echo "Open the DMG to confirm the branded window, then ship it."
