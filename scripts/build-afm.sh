#!/usr/bin/env bash
# build-afm — compile PortBay's Apple Foundation Models bridge (Smart
# Dictation's zero-setup rewrite provider, src-tauri/afm/main.swift) and
# place it where Tauri's bundler expects an external binary:
#
#   src-tauri/binaries/portbay-afm-<rust-target-triple>
#
# macOS-only: the sidecar wraps the Swift-only FoundationModels framework
# (macOS 26's on-device LLM), so it is listed in tauri.macos.conf.json's
# externalBin — NOT the base tauri.conf.json — and Linux bundles never look
# for it. On Linux this script is a no-op so shared CI/dev flows can call it
# unconditionally.
#
# Deployment target is macOS 13 (not the app's 11): the binary uses Swift
# concurrency, whose runtime ships in the OS from 12+. On an older host the
# exec fails and the app keeps the raw transcript — the same degrade as any
# other rewrite failure, and Apple Intelligence needs macOS 26 regardless.
#
# Run before `tauri build` / `tauri dev`. Idempotent.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "build-afm: not macOS; skipping (FoundationModels sidecar is macOS-only)"
  exit 0
fi

case "${TARGET_TRIPLE:-$(uname -m)}" in
  aarch64-apple-darwin | arm64) triple="aarch64-apple-darwin" arch="arm64" ;;
  x86_64-apple-darwin | x86_64) triple="x86_64-apple-darwin" arch="x86_64" ;;
  *)
    echo "build-afm: unsupported target ${TARGET_TRIPLE:-$(uname -m)}" >&2
    exit 1
    ;;
esac

mkdir -p "$bin_dir"
dest="${bin_dir}/portbay-afm-${triple}"

# Same chicken-and-egg seeding as build-hosts-helper.sh: tauri_build's
# externalBin existence check runs before any of our own sidecars exist.
for ours in portbay-hosts-helper portbay-mcp portbay-afm; do
  ph="${bin_dir}/${ours}-${triple}"
  [ -f "$ph" ] || : >"$ph"
done

echo "build-afm: swiftc -O -target ${arch}-apple-macos13.0 -> ${dest}"
swiftc -O -target "${arch}-apple-macos13.0" \
  "${repo_root}/src-tauri/afm/main.swift" \
  -o "$dest"
chmod +x "$dest"

echo "build-afm: ✓ ${dest}"
