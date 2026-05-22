#!/usr/bin/env bash
# fetch-mkcert — download the mkcert binary into src-tauri/binaries/
#
# Same shape as fetch-caddy.sh. Tauri's sidecar convention expects the
# binary at src-tauri/binaries/<name>-<rust-target-triple> so that
# `app.shell().sidecar("mkcert")` resolves correctly on each platform.
#
# Re-run after bumping MKCERT_VERSION. Idempotent.

set -euo pipefail

MKCERT_VERSION="${MKCERT_VERSION:-1.4.4}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s-$uname_m" in
  Darwin-arm64)
    asset="mkcert-v${MKCERT_VERSION}-darwin-arm64"
    triple="aarch64-apple-darwin"
    ;;
  Darwin-x86_64)
    asset="mkcert-v${MKCERT_VERSION}-darwin-amd64"
    triple="x86_64-apple-darwin"
    ;;
  Linux-x86_64)
    asset="mkcert-v${MKCERT_VERSION}-linux-amd64"
    triple="x86_64-unknown-linux-gnu"
    ;;
  Linux-aarch64)
    asset="mkcert-v${MKCERT_VERSION}-linux-arm64"
    triple="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "fetch-mkcert: unsupported host $uname_s-$uname_m" >&2
    exit 1
    ;;
esac

url="https://github.com/FiloSottile/mkcert/releases/download/v${MKCERT_VERSION}/${asset}"

mkdir -p "$bin_dir"
dest="${bin_dir}/mkcert-${triple}"

echo "fetch-mkcert: downloading ${url}"
curl -fL --retry 3 -o "$dest" "$url"
chmod +x "$dest"

if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-mkcert: ✓ ${dest}"
"$dest" -version 2>/dev/null || "$dest" --version
