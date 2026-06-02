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

# Supply-chain: verify against a SHA-256 pinned in-repo before bundling. Fail
# closed on mismatch. Recompute on a CLOUDFLARED_VERSION bump with:
# shasum -a 256 "${tmp}/${asset}". Linux unpinned (not shipped) → fails closed.
case "$triple" in
  aarch64-apple-darwin) want_sha="45cfbb59a720f60b873906aa6469f8c4058f26be6d351c3e2920bc9cb4714273" ;;
  x86_64-apple-darwin)  want_sha="155a288fef19dba08f0c7145c16a207baf137462d8a1289a78bf8564f9e51244" ;;
  x86_64-unknown-linux-gnu) want_sha="991dffd8889ee9f0147b6b48933da9e4407e68ea8c6d984f55fa2d3db4bb431d" ;;
  *) echo "fetch-cloudflared: no pinned sha256 for $triple (cloudflared $CLOUDFLARED_VERSION); add one before building this arch" >&2; exit 1 ;;
esac
got_sha="$(shasum -a 256 "${tmp}/${asset}" | cut -d' ' -f1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "fetch-cloudflared: SHA-256 mismatch for ${asset} ($triple)" >&2
  echo "  expected ${want_sha}" >&2
  echo "  got      ${got_sha}" >&2
  exit 1
fi
echo "fetch-cloudflared: ✓ sha256 verified"

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
