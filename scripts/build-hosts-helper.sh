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

# Chicken-and-egg: tauri.conf.json lists this path as an externalBin, and
# `tauri_build` (run from the crate's build.rs) validates that every externalBin
# file EXISTS at compile time. But the helper IS part of that same crate, so
# building it would fail on its own missing externalBin. Seed an empty
# placeholder first to satisfy the existence check, then overwrite it with the
# real binary once the build succeeds.
[ -f "$dest" ] || : > "$dest"

echo "build-hosts-helper: cargo build --release --target ${triple} --bin portbay-hosts-helper"
cargo build --release --manifest-path "$manifest" --target "$triple" --bin portbay-hosts-helper

src="${repo_root}/src-tauri/target/${triple}/release/portbay-hosts-helper"
cp "$src" "$dest"
chmod +x "$dest"

echo "build-hosts-helper: ✓ ${dest}"
