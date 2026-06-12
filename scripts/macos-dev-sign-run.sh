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
	<key>CFBundleIconFile</key><string>icon.icns</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>NSHighResolutionCapable</key><true/>
	<!-- TCC usage descriptions, mirrored from src-tauri/Info.plist. Since the
	     runner launches via \`open\`, this bundle is the TCC-responsible
	     process for the app AND its sidecars — without the mic key, macOS
	     kills portbay-stt outright the moment it opens the microphone
	     ("Speech engine error" in the notch). -->
	<key>NSMicrophoneUsageDescription</key><string>PortBay uses the microphone when you dictate text into a task card via macOS Dictation, and to record narration in screen recordings.</string>
	<key>NSSpeechRecognitionUsageDescription</key><string>PortBay uses macOS speech recognition to turn your dictation into text on task cards.</string>
	<key>NSAppleEventsUsageDescription</key><string>PortBay opens and drives your terminal app to launch agent tasks you dispatch from the board.</string>
	<key>NSCameraUsageDescription</key><string>PortBay uses the camera for the webcam overlay in screen recordings.</string>
</dict>
</plist>
EOF

# Real app icon: without it NSRunningApplication.icon serves the generic
# blank-document glyph — which the dictation notch then shows as a white box
# whenever the dictation target is PortBay itself. (Paths are src-tauri-
# relative, same as tauri.conf.json above.)
mkdir -p "$app/Contents/Resources"
cp -f icons/icon.icns "$app/Contents/Resources/icon.icns" 2>/dev/null \
  || echo "macos-dev-sign-run: icons/icon.icns missing; dev bundle keeps the generic icon" >&2

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

# Launch through LaunchServices (`open`) instead of exec'ing the binary.
# A directly-exec'd process stays parented to the terminal session and
# RunningBoard never grants it the foreground-application role, so system
# services that gate on that role refuse it even when it is visually
# frontmost — Apple Image Playground's ImageCreator throws
# `backgroundCreationForbidden` ("Acquiring VisualGeneration to deny
# background requests"). Verified live 2026-06-10: the same signed bundle
# passes the gate when `open`ed and is refused when exec'd.
#
# `tauri dev` / `cargo run` still need the runner to behave like the app
# process: stay alive while it runs, surface its stdout/stderr, forward the
# dev environment, and take the app down when the runner is killed (rebuild,
# Ctrl-C). FIFOs bridge stdio, `--env` forwards the environment, and traps
# forward termination to the app process.
stdio_dir="$(mktemp -d)"
mkfifo "$stdio_dir/out" "$stdio_dir/err"
cat "$stdio_dir/out" &
out_pid=$!
cat "$stdio_dir/err" >&2 &
err_pid=$!

env_args=()
while IFS= read -r -d '' kv; do
  env_args+=(--env "$kv")
done < <(env -0)

extra_args=()
if (($#)); then
  extra_args=(--args "$@")
fi

app_pid=""
cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "$app_pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -9 "$app_pid" 2>/dev/null || true
  fi
  kill "$out_pid" "$err_pid" 2>/dev/null || true
  rm -rf "$stdio_dir"
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

# `${arr[@]+...}` guards: macOS's bash 3.2 treats expanding an empty array as
# an unbound variable under `set -u`, and extra_args is empty on a plain
# `tauri dev` launch.
/usr/bin/open -n -W --stdout "$stdio_dir/out" --stderr "$stdio_dir/err" \
  ${env_args[@]+"${env_args[@]}"} "$app" ${extra_args[@]+"${extra_args[@]}"} &
open_pid=$!

# Newest instance of this bundle's executable = the one `open -n` just spawned
# (an old instance may still be shutting down during a dev restart).
for _ in $(seq 1 100); do
  app_pid="$(pgrep -n -f "$macos_dir/portbay-app" 2>/dev/null || true)"
  [[ -n "$app_pid" ]] && break
  sleep 0.1
done

wait "$open_pid"
