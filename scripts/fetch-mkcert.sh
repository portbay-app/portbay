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

# Supply-chain: verify against a SHA-256 pinned in-repo before bundling (mkcert
# ships a raw binary, no upstream checksum file). Fail closed on mismatch.
# Recompute on a MKCERT_VERSION bump with: shasum -a 256 "$dest". Linux unpinned
# (not shipped) → fails closed.
case "$triple" in
  aarch64-apple-darwin) want_sha="c8af0df44bce04359794dad8ea28d750437411d632748049d08644ffb66a60c6" ;;
  x86_64-apple-darwin)  want_sha="a32dfab51f1845d51e810db8e47dcf0e6b51ae3422426514bf5a2b8302e97d4e" ;;
  x86_64-unknown-linux-gnu) want_sha="6d31c65b03972c6dc4a14ab429f2928300518b26503f58723e532d1b0a3bbb52" ;;
  *) echo "fetch-mkcert: no pinned sha256 for $triple (mkcert $MKCERT_VERSION); add one before building this arch" >&2; exit 1 ;;
esac
got_sha="$(shasum -a 256 "$dest" | cut -d' ' -f1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "fetch-mkcert: SHA-256 mismatch for ${asset} ($triple)" >&2
  echo "  expected ${want_sha}" >&2
  echo "  got      ${got_sha}" >&2
  rm -f "$dest"
  exit 1
fi
echo "fetch-mkcert: ✓ sha256 verified"
chmod +x "$dest"

if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-mkcert: ✓ ${dest}"
"$dest" -version 2>/dev/null || "$dest" --version
