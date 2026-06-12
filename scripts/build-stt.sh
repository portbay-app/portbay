#!/usr/bin/env bash
# build-stt — build PortBay's local speech-to-text sidecar (Smart Dictation's
# on-device transcription engine, src-tauri/stt/) and place it where Tauri's
# bundler expects an external binary:
#
#   src-tauri/binaries/portbay-stt-<rust-target-triple>
#
# Unlike build-afm.sh's bare swiftc, this is a SwiftPM release build — the
# sidecar depends on WhisperKit and FluidAudio (CoreML speech models), which
# are SwiftPM packages. The first build clones and compiles both (slow);
# SwiftPM caches make rebuilds incremental.
#
# macOS-only: both engine libraries are Swift/CoreML (floor macOS 14), so the
# sidecar is listed in tauri.macos.conf.json's externalBin — NOT the base
# tauri.conf.json — and Linux bundles never look for it. On Linux this script
# is a no-op so shared CI/dev flows can call it unconditionally.
#
# Run before `tauri build` / `tauri dev`. Idempotent.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "build-stt: not macOS; skipping (speech-to-text sidecar is macOS-only)"
  exit 0
fi

case "${TARGET_TRIPLE:-$(uname -m)}" in
  aarch64-apple-darwin | arm64) triple="aarch64-apple-darwin" arch="arm64" ;;
  x86_64-apple-darwin | x86_64) triple="x86_64-apple-darwin" arch="x86_64" ;;
  *)
    echo "build-stt: unsupported target ${TARGET_TRIPLE:-$(uname -m)}" >&2
    exit 1
    ;;
esac

mkdir -p "$bin_dir"
dest="${bin_dir}/portbay-stt-${triple}"

# Same chicken-and-egg seeding as build-afm.sh: tauri_build's externalBin
# existence check runs before any of our own sidecars exist.
for ours in portbay-hosts-helper portbay-mcp portbay-afm portbay-stt portbay-imagegen portbay-capture; do
  ph="${bin_dir}/${ours}-${triple}"
  [ -f "$ph" ] || : >"$ph"
done

echo "build-stt: swift build -c release (${arch}) -> ${dest}"
swift build \
  --package-path "${repo_root}/src-tauri/stt" \
  -c release \
  --arch "${arch}"

built="$(swift build --package-path "${repo_root}/src-tauri/stt" -c release --arch "${arch}" --show-bin-path)/portbay-stt"
cp -f "$built" "$dest"
chmod +x "$dest"

echo "build-stt: ✓ ${dest}"
