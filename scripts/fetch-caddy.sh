#!/usr/bin/env bash
# fetch-caddy — build the Caddy server binary into src-tauri/binaries/
#
# Tauri's sidecar convention expects the binary at
#   src-tauri/binaries/<name>-<rust-target-triple>
# so that `app.shell().sidecar("caddy")` resolves to
# `caddy-aarch64-apple-darwin` on Apple Silicon, `caddy-x86_64-apple-darwin`
# on Intel, etc. This script picks the right asset for the host and drops
# it into place. PortBay needs the Cloudflare DNS provider for Public ACME
# wildcard certificates, so this builds a custom Caddy with that module.
#
# Re-run after bumping CADDY_VERSION below. Idempotent — replaces the
# existing binary if one is already in place.

set -euo pipefail

CADDY_VERSION="${CADDY_VERSION:-2.10.2}"
XCADDY_VERSION="${XCADDY_VERSION:-v0.4.4}"
CADDY_CLOUDFLARE_VERSION="${CADDY_CLOUDFLARE_VERSION:-v0.2.3}"

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

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if ! command -v go >/dev/null 2>&1; then
  echo "fetch-caddy: Go is required to build Caddy with github.com/caddy-dns/cloudflare" >&2
  exit 1
fi

mkdir -p "$bin_dir"
dest="${bin_dir}/caddy-${triple}"

echo "fetch-caddy: building Caddy v${CADDY_VERSION} with github.com/caddy-dns/cloudflare@${CADDY_CLOUDFLARE_VERSION}"
GOBIN="${tmp}/bin" go install "github.com/caddyserver/xcaddy/cmd/xcaddy@${XCADDY_VERSION}"
"${tmp}/bin/xcaddy" build "v${CADDY_VERSION}" \
  --with "github.com/caddy-dns/cloudflare@${CADDY_CLOUDFLARE_VERSION}" \
  --output "$dest"
chmod +x "$dest"

# macOS Gatekeeper: strip the quarantine xattr so the bundled sidecar
# doesn't get blocked on first launch in dev.
if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-caddy: ✓ ${dest}"
"$dest" version
