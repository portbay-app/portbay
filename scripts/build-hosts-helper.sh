#!/usr/bin/env bash
# build-hosts-helper — compile PortBay's privileged hosts helper from source
# and place it where Tauri's bundler expects an external binary:
#
#   src-tauri/binaries/portbay-hosts-helper-<rust-target-triple>
#
# Unlike the fetch-*.sh scripts (which download third-party sidecars), the
# helper is OUR code (src-tauri/src/bin/portbay-hosts-helper.rs), so we build it
# rather than download it. It MUST be bundled next to the app binary: at
# runtime `commands::dnsmasq::resolve_helper_bin` looks for the helper beside
# the running executable, and `hosts_helper::install_daemon` copies it to
# /usr/local/bin under one admin prompt. Without this step the packaged .app has
# no helper to install, so the /etc/hosts (and resolver) writes silently can't
# happen on a fresh machine.
#
# Run before `tauri build`. Idempotent.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/src-tauri/Cargo.toml"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s-$uname_m" in
  Darwin-arm64)   host_triple="aarch64-apple-darwin" ;;
  Darwin-x86_64)  host_triple="x86_64-apple-darwin" ;;
  Linux-x86_64)   host_triple="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  host_triple="aarch64-unknown-linux-gnu" ;;
  *)
    echo "build-hosts-helper: unsupported host $uname_s-$uname_m" >&2
    exit 1
    ;;
esac

# CI cross-builds pass an explicit target to `tauri build`; match it here so the
# bundled helper is the right architecture. Defaults to the host triple.
triple="${TARGET_TRIPLE:-$host_triple}"

mkdir -p "$bin_dir"
dest="${bin_dir}/portbay-hosts-helper-${triple}"

# Chicken-and-egg: tauri.conf.json lists our own binaries as externalBin, and
# `tauri_build` (run from the app crate's build.rs) validates that EVERY
# externalBin file exists at compile time. But building any of our own binaries
# compiles that build.rs, which would fail on the others (and on this one)
# before they exist. Seed empty placeholders for all of our own sidecars first
# to satisfy the existence check; the real binary overwrites this one below,
# build-mcp.sh overwrites portbay-mcp, and build-afm.sh overwrites portbay-afm
# (macOS-only — on Linux the placeholder is harmless and never bundled, since
# portbay-afm is listed only in tauri.macos.conf.json).
for ours in portbay-hosts-helper portbay-mcp portbay-afm; do
  ph="${bin_dir}/${ours}-${triple}"
  [ -f "$ph" ] || : > "$ph"
done

echo "build-hosts-helper: cargo build --release --target ${triple} -p portbay-hosts-helper"
cargo build --release --manifest-path "$manifest" --target "$triple" -p portbay-hosts-helper

src="${repo_root}/src-tauri/target/${triple}/release/portbay-hosts-helper"
cp "$src" "$dest"
chmod +x "$dest"

echo "build-hosts-helper: ✓ ${dest}"
