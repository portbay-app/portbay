#!/usr/bin/env bash
# build-mcp — compile PortBay's MCP server from source and place it where
# Tauri's bundler expects an external binary:
#
#   src-tauri/binaries/portbay-mcp-<rust-target-triple>
#
# Like build-hosts-helper.sh, this is OUR code (src-tauri/crates/mcp), so we
# build it rather than download it. It ships inside PortBay.app as a sidecar so
# MCP-aware agents (Claude Code, Cursor, Continue, Zed, …) can spawn it to drive
# PortBay over stdio. Without this step the packaged .app has no MCP server to
# advertise.
#
# Lives in its own workspace crate (crates/mcp) so the rmcp/schemars stack is
# compiled only here, never for the GUI. Run before `tauri build`. Idempotent.

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
    echo "build-mcp: unsupported host $uname_s-$uname_m" >&2
    exit 1
    ;;
esac

# CI cross-builds pass an explicit target to `tauri build`; match it here so the
# bundled server is the right architecture. Defaults to the host triple.
triple="${TARGET_TRIPLE:-$host_triple}"

mkdir -p "$bin_dir"
dest="${bin_dir}/portbay-mcp-${triple}"

# Chicken-and-egg: tauri.conf.json lists our own binaries as externalBin, and
# `tauri_build` (run from the app crate's build.rs, which compiles as a
# dependency of crates/mcp) validates that EVERY externalBin file exists at
# compile time. Seed empty placeholders for all of our own sidecars first to
# satisfy the existence check; the real binary overwrites this one below, and
# build-hosts-helper.sh overwrites portbay-hosts-helper (and build-afm.sh
# overwrites portbay-afm on macOS; the Linux placeholder is never bundled).
for ours in portbay-hosts-helper portbay-mcp portbay-afm; do
  ph="${bin_dir}/${ours}-${triple}"
  [ -f "$ph" ] || : > "$ph"
done

# Light up the proprietary task board in the sidecar when the desktop-pro
# overlay is present in this checkout (same source the GUI/CLI gate behind
# `tasks`). The public OSS checkout lacks src/context, so the released sidecar
# stays board-free without any flag here. Mirrors scripts/dev-pro.sh.
feature_args=()
if [ -f "${repo_root}/src-tauri/src/context/board.rs" ]; then
  feature_args=(--features tasks)
  echo "build-mcp: desktop-pro overlay detected — building with --features tasks"
fi

echo "build-mcp: cargo build --release --target ${triple} -p portbay-mcp ${feature_args[*]}"
cargo build --release --manifest-path "$manifest" --target "$triple" -p portbay-mcp "${feature_args[@]}"

src="${repo_root}/src-tauri/target/${triple}/release/portbay-mcp"
cp "$src" "$dest"
chmod +x "$dest"

echo "build-mcp: ✓ ${dest}"
