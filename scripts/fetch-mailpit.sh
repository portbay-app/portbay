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

# Supply-chain: verify against a SHA-256 pinned in-repo before bundling. Fail
# closed on mismatch. Recompute on a MAILPIT_VERSION bump with:
# shasum -a 256 "${tmp}/${asset}". Linux unpinned (not shipped) → fails closed.
case "$triple" in
  aarch64-apple-darwin) want_sha="dbebbf3e95e82e111dd08fbec106cc09c026b207a9f105e45f212db4c63824a5" ;;
  x86_64-apple-darwin)  want_sha="066d4e8e9bcb0a9a6ce1a298991064eb4986010909e3b6460648ab8724b543f5" ;;
  *) echo "fetch-mailpit: no pinned sha256 for $triple (mailpit $MAILPIT_VERSION); add one before building this arch" >&2; exit 1 ;;
esac
got_sha="$(shasum -a 256 "${tmp}/${asset}" | cut -d' ' -f1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "fetch-mailpit: SHA-256 mismatch for ${asset} ($triple)" >&2
  echo "  expected ${want_sha}" >&2
  echo "  got      ${got_sha}" >&2
  exit 1
fi
echo "fetch-mailpit: ✓ sha256 verified"

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
# Best-effort smoke print only. `mailpit version` performs an online
# "latest release" check that returns 403 on unauthenticated CI, and
# mailpit has no `--version` flag — neither may fail the fetch. The
# successful download + extract above is the real success signal.
"$dest" version 2>/dev/null || true
