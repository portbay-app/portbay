#!/usr/bin/env bash
# fetch-cloudflared — download the cloudflared binary into
# src-tauri/binaries/. Same shape as fetch-caddy.sh / fetch-mailpit.sh.

set -euo pipefail

CLOUDFLARED_VERSION="${CLOUDFLARED_VERSION:-2025.11.1}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

# cloudflared ships per-OS/arch binaries directly (no tarball). The
# naming follows `cloudflared-<os>-<arch>` on GitHub releases.
case "$uname_s-$uname_m" in
  Darwin-arm64)
    asset="cloudflared-darwin-arm64.tgz"
    triple="aarch64-apple-darwin"
    inner="cloudflared"
    is_tgz=1
    ;;
  Darwin-x86_64)
    asset="cloudflared-darwin-amd64.tgz"
    triple="x86_64-apple-darwin"
    inner="cloudflared"
    is_tgz=1
    ;;
  Linux-x86_64)
    asset="cloudflared-linux-amd64"
    triple="x86_64-unknown-linux-gnu"
    inner=""
    is_tgz=0
    ;;
  Linux-aarch64)
    asset="cloudflared-linux-arm64"
    triple="aarch64-unknown-linux-gnu"
    inner=""
    is_tgz=0
    ;;
  *)
    echo "fetch-cloudflared: unsupported host $uname_s-$uname_m" >&2
    exit 1
    ;;
esac

url="https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$bin_dir"
dest="${bin_dir}/cloudflared-${triple}"

echo "fetch-cloudflared: downloading ${url}"
curl -fL --retry 3 -o "${tmp}/${asset}" "$url"

if [[ "$is_tgz" -eq 1 ]]; then
  tar -xzf "${tmp}/${asset}" -C "$tmp"
  mv "${tmp}/${inner}" "$dest"
else
  mv "${tmp}/${asset}" "$dest"
fi
chmod +x "$dest"

if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-cloudflared: ✓ ${dest}"
"$dest" --version 2>/dev/null || true
