#!/usr/bin/env bash
# Cargo runner for macOS dev builds: launch the freshly built app binary from
# inside a minimal .app bundle, re-signed with a STABLE code identity.
#
# Why the bundle (root-caused live, 2026-06-06): macOS never brings up the
# dictation speech recognizer for an IMK client that has no bundle identity.
# The bare `target/debug/portbay-app` that `tauri dev` spawns registers with
# DictationIM as an anonymous numeric input controller (e.g. `802425801.447`)
# instead of a bundle id, and every dictation session stalls at stage 1 —
# "start listening" fires, `dictation is allowed`, then the recognizer never
# starts and the session exits. The same binary launched from inside a
# Contents/MacOS/ wrapper registers as `com.portbay-app.portbay` and reaches
# stage 2 ("Recognizer start listening") immediately. assistantd's
# `kAFAssistantErrorDomain Code=41` is a red herring — it fires in healthy
# sessions too.
#
# Why the signature: cargo's default ad-hoc (linker) signature embeds a
# per-build hash in the identifier (e.g. `portbay_app-d299e2670db0ae61`), so
# every rebuild looks like a brand-new app to macOS and per-app consent state
# (dictation onboarding, keychain ACLs) resets. Signing with a real identity
# + the bundle identifier keeps the identity stable across rebuilds, exactly
# like the packaged app.
#
# Wired via [target] runner in src-tauri/.cargo/config.toml, so `tauri dev`,
# `cargo run`, and `cargo test` all pass through here (verified: tauri dev
# DOES route through the cargo runner; the launched pid is the runner's own
# after exec). Paths are resolved relative to src-tauri (where cargo runs).
#
# Only the app binary itself gets the bundle treatment — test binaries and
# other workspace bins are signed (when possible) and exec'd as before.
# No usable identity (CI, contributors without certs) → the bundle is still
# built (bundle identity alone may matter) but the binary runs with its
# ad-hoc signature, exactly as before. Override the identity with
# PORTBAY_DEV_SIGN_IDENTITY.
set -euo pipefail

bin="$1"
shift

identity="${PORTBAY_DEV_SIGN_IDENTITY:-Apple Development}"
identifier="com.portbay-app.portbay"

if [[ "$(uname)" != "Darwin" ]]; then
  exec "$bin" "$@"
fi

have_identity=0
if /usr/bin/security find-identity -v -p codesigning 2>/dev/null | grep -q "$identity"; then
  have_identity=1
fi

sign() {
  # --force replaces the ad-hoc linker signature. Failure (locked keychain,
  # expired cert) must never block a dev run — fall through unsigned.
  [[ "$have_identity" == 1 ]] || return 0
  /usr/bin/codesign --force --sign "$identity" --identifier "$identifier" "$1" \
    || echo "macos-dev-sign-run: codesign failed; running unsigned" >&2
}

# Non-app binaries (tests, the CLI, benches): sign in place and run directly.
if [[ "$(basename "$bin")" != "portbay-app" ]]; then
  sign "$bin"
  exec "$bin" "$@"
fi

# ---- App binary: rebuild the .app shim around it, then exec through it ----
bin_dir="$(cd "$(dirname "$bin")" && pwd)"
app="$bin_dir/PortBay-dev.app"
macos_dir="$app/Contents/MacOS"
mkdir -p "$macos_dir"

version="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' tauri.conf.json 2>/dev/null | head -1)"
version="${version:-0.0.0-dev}"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key><string>portbay-app</string>
	<key>CFBundleIdentifier</key><string>$identifier</string>
	<key>CFBundleName</key><string>PortBay</string>
	<key>CFBundleDisplayName</key><string>PortBay</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$version</string>
	<key>CFBundleVersion</key><string>$version</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Fresh copy of the just-built binary (the bundle's main executable must be a
# real file for codesign; ~100 MB copy is sub-second on the dev SSD).
cp -f "$bin" "$macos_dir/portbay-app"

# Sidecars and the CLI resolve relative to current_exe(), which now lives in
# Contents/MacOS/ — hard-link every top-level executable/dylib next to it.
# Hard links, not symlinks: Tauri's StartingBinary rejects current_exe paths
# containing symlinks, and links must be refreshed per launch anyway (a
# rebuilt sidecar is a new inode).
for f in "$bin_dir"/*; do
  name="$(basename "$f")"
  [[ -f "$f" ]] || continue
  [[ "$name" == "portbay-app" ]] && continue
  [[ -x "$f" || "$name" == *.dylib ]] || continue
  ln -f "$f" "$macos_dir/$name" 2>/dev/null \
    || cp -f "$f" "$macos_dir/$name"
done

sign "$app"

exec "$macos_dir/portbay-app" "$@"
