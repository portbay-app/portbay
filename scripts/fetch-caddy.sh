#!/usr/bin/env bash
# fetch-caddy — download the Caddy server binary into src-tauri/binaries/
#
# Tauri's sidecar convention expects the binary at
#   src-tauri/binaries/<name>-<rust-target-triple>
# so that `app.shell().sidecar("caddy")` resolves to
# `caddy-aarch64-apple-darwin` on Apple Silicon, `caddy-x86_64-apple-darwin`
# on Intel, etc. This script picks the right asset for the host and drops
# it into place.
#
# Re-run after bumping CADDY_VERSION below. Idempotent — replaces the
# existing binary if one is already in place.

set -euo pipefail

CADDY_VERSION="${CADDY_VERSION:-2.8.4}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s" in
  Darwin) os="mac" ;;
  Linux)  os="linux" ;;
  *) echo "fetch-caddy: unsupported OS '$uname_s'" >&2; exit 1 ;;
esac

case "$uname_m" in
  arm64|aarch64) arch="arm64" ;;
  x86_64)        arch="amd64" ;;
  *) echo "fetch-caddy: unsupported arch '$uname_m'" >&2; exit 1 ;;
esac

case "$uname_s-$uname_m" in
  Darwin-arm64)   triple="aarch64-apple-darwin" ;;
  Darwin-x86_64)  triple="x86_64-apple-darwin" ;;
  Linux-x86_64)   triple="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  triple="aarch64-unknown-linux-gnu" ;;
  *) echo "fetch-caddy: no rust-triple mapping for $uname_s-$uname_m" >&2; exit 1 ;;
esac

archive="caddy_${CADDY_VERSION}_${os}_${arch}.tar.gz"
url="https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/${archive}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetch-caddy: downloading ${url}"
curl -fL --retry 3 -o "${tmp}/${archive}" "$url"

# Supply-chain: verify the download against a SHA-256 pinned in-repo before we
# trust the binary enough to bundle it in the signed .app. A mismatch means the
# download was corrupted or tampered with — fail closed. On a CADDY_VERSION bump,
# recompute with: shasum -a 256 "${tmp}/${archive}". Linux is intentionally
# unpinned (not shipped yet) and fails closed with guidance.
case "$triple" in
  aarch64-apple-darwin) want_sha="8445900771cb8455d1dee729486d692c6a60519bb682322270b32d1d602d37dc" ;;
  x86_64-apple-darwin)  want_sha="d566885bfed36198f980fdefa63aabdb18f1e3264a0e3121ad9f028de851f311" ;;
  *) echo "fetch-caddy: no pinned sha256 for $triple (caddy $CADDY_VERSION); add one before building this arch" >&2; exit 1 ;;
esac
got_sha="$(shasum -a 256 "${tmp}/${archive}" | cut -d' ' -f1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "fetch-caddy: SHA-256 mismatch for ${archive} ($triple)" >&2
  echo "  expected ${want_sha}" >&2
  echo "  got      ${got_sha}" >&2
  exit 1
fi
echo "fetch-caddy: ✓ sha256 verified"

echo "fetch-caddy: extracting"
tar -xzf "${tmp}/${archive}" -C "$tmp" caddy

mkdir -p "$bin_dir"
dest="${bin_dir}/caddy-${triple}"
mv "${tmp}/caddy" "$dest"
chmod +x "$dest"

# macOS Gatekeeper: strip the quarantine xattr so the bundled sidecar
# doesn't get blocked on first launch in dev.
if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-caddy: ✓ ${dest}"
"$dest" version
