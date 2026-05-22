#!/usr/bin/env bash
# fetch-mailpit — download the Mailpit binary into src-tauri/binaries/
#
# Same shape as fetch-caddy.sh / fetch-mkcert.sh. Bundles the upstream
# release tarball into the Tauri sidecar slot at
# src-tauri/binaries/mailpit-<rust-target-triple>.

set -euo pipefail

MAILPIT_VERSION="${MAILPIT_VERSION:-1.30.0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s-$uname_m" in
  Darwin-arm64)
    asset="mailpit-darwin-arm64.tar.gz"
    triple="aarch64-apple-darwin"
    ;;
  Darwin-x86_64)
    asset="mailpit-darwin-amd64.tar.gz"
    triple="x86_64-apple-darwin"
    ;;
  Linux-x86_64)
    asset="mailpit-linux-amd64.tar.gz"
    triple="x86_64-unknown-linux-gnu"
    ;;
  Linux-aarch64)
    asset="mailpit-linux-arm64.tar.gz"
    triple="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "fetch-mailpit: unsupported host $uname_s-$uname_m" >&2
    exit 1
    ;;
esac

url="https://github.com/axllent/mailpit/releases/download/v${MAILPIT_VERSION}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetch-mailpit: downloading ${url}"
curl -fL --retry 3 -o "${tmp}/${asset}" "$url"

echo "fetch-mailpit: extracting"
tar -xzf "${tmp}/${asset}" -C "$tmp" mailpit

mkdir -p "$bin_dir"
dest="${bin_dir}/mailpit-${triple}"
mv "${tmp}/mailpit" "$dest"
chmod +x "$dest"

if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-mailpit: ✓ ${dest}"
"$dest" version 2>/dev/null || "$dest" --version
