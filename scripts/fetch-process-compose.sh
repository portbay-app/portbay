#!/usr/bin/env bash
# fetch-process-compose — download the process-compose binary into
# src-tauri/binaries/
#
# process-compose is the supervisor PortBay drives over its REST API; it
# ships as a Tauri sidecar at
#   src-tauri/binaries/process-compose-<rust-target-triple>
# so `app.shell().sidecar("process-compose")` resolves on each target.
# This script picks the right GitHub release asset for the host and drops
# it into place.
#
# Re-run after bumping PROCESS_COMPOSE_VERSION below. Idempotent —
# replaces the existing binary if one is already in place.

set -euo pipefail

PROCESS_COMPOSE_VERSION="${PROCESS_COMPOSE_VERSION:-1.110.0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s" in
  Darwin) os="darwin" ;;
  Linux)  os="linux" ;;
  *) echo "fetch-process-compose: unsupported OS '$uname_s'" >&2; exit 1 ;;
esac

case "$uname_m" in
  arm64|aarch64) arch="arm64" ;;
  x86_64)        arch="amd64" ;;
  *) echo "fetch-process-compose: unsupported arch '$uname_m'" >&2; exit 1 ;;
esac

case "$uname_s-$uname_m" in
  Darwin-arm64)   triple="aarch64-apple-darwin" ;;
  Darwin-x86_64)  triple="x86_64-apple-darwin" ;;
  Linux-x86_64)   triple="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  triple="aarch64-unknown-linux-gnu" ;;
  *) echo "fetch-process-compose: no rust-triple mapping for $uname_s-$uname_m" >&2; exit 1 ;;
esac

archive="process-compose_${os}_${arch}.tar.gz"
url="https://github.com/F1bonacc1/process-compose/releases/download/v${PROCESS_COMPOSE_VERSION}/${archive}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetch-process-compose: downloading ${url}"
curl -fL --retry 3 -o "${tmp}/${archive}" "$url"

echo "fetch-process-compose: extracting"
# Tarball layout: LICENSE, README.md, process-compose
tar -xzf "${tmp}/${archive}" -C "$tmp" process-compose

mkdir -p "$bin_dir"
dest="${bin_dir}/process-compose-${triple}"
mv "${tmp}/process-compose" "$dest"
chmod +x "$dest"

# macOS Gatekeeper: strip the quarantine xattr so the bundled sidecar
# doesn't get blocked on first launch in dev.
if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-process-compose: ✓ ${dest}"
"$dest" version | head -n 2
