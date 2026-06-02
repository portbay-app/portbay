#!/usr/bin/env bash
# Build a local Linux AppImage for smoke testing. Run on Ubuntu 22.04/24.04
# with the Tauri Linux packages installed.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *)
    echo "release-appimage-local: run on Linux x86_64" >&2
    exit 1
    ;;
esac

command -v pnpm >/dev/null 2>&1 || {
  echo "release-appimage-local: pnpm is required" >&2
  exit 1
}

for bin in dnsmasq notify-send pkexec systemctl; do
  command -v "$bin" >/dev/null 2>&1 || {
    echo "release-appimage-local: missing required command: $bin" >&2
    exit 1
  }
done

./scripts/fetch-process-compose.sh
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
./scripts/fetch-dnsmasq.sh
./scripts/build-hosts-helper.sh
./scripts/build-mcp.sh

pnpm install --frozen-lockfile
pnpm tauri build --bundles appimage

echo "release-appimage-local: artifacts under src-tauri/target/release/bundle/appimage"
echo "release-appimage-local: use scripts/release-linux-local.sh for AppImage + deb + rpm"
