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

# Supply-chain: verify against a SHA-256 pinned in-repo before bundling. Fail
# closed on mismatch. Recompute on a PROCESS_COMPOSE_VERSION bump with:
# shasum -a 256 "${tmp}/${archive}". Linux unpinned (not shipped) → fails closed.
case "$triple" in
  aarch64-apple-darwin) want_sha="4abc00e402bee5a700e3ec1c94ffda2fe73b414866286a59134d81f372595ebb" ;;
  x86_64-apple-darwin)  want_sha="1101270e1ac63e02e9f97ef834a3b8387d4e6641682366ac193de466a2d1747e" ;;
  x86_64-unknown-linux-gnu) want_sha="945a9d9494cdc6daa0ea7c121c23cc2cb1a0e1877db487c6840a705be0b4d01c" ;;
  *) echo "fetch-process-compose: no pinned sha256 for $triple (process-compose $PROCESS_COMPOSE_VERSION); add one before building this arch" >&2; exit 1 ;;
esac
got_sha="$(shasum -a 256 "${tmp}/${archive}" | cut -d' ' -f1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "fetch-process-compose: SHA-256 mismatch for ${archive} ($triple)" >&2
  echo "  expected ${want_sha}" >&2
  echo "  got      ${got_sha}" >&2
  exit 1
fi
echo "fetch-process-compose: ✓ sha256 verified"

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
# Best-effort smoke print; the successful download + extract is the real
# success signal, so never let the version probe fail the fetch.
"$dest" version 2>/dev/null | head -n 2 || true
