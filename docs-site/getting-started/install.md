---
title: Install PortBay — Homebrew, DMG, Linux & Build from Source
description: Install PortBay on macOS or Linux, download release bundles, or build from source with Tauri — full requirements and sidecar fetch steps included.
---

# Install

PortBay ships as a signed macOS app today. Linux desktop builds are being added
as AppImage, deb, rpm, Snap, and AUR packages; see
[Linux Support](/getting-started/linux) for the current requirements and
limitations.

## Homebrew

```bash
brew tap portbay-app/portbay
brew install --cask portbay
```

The cask installs `PortBay.app` and the bundled `portbay` CLI. Uninstalling with Homebrew removes the app; `brew uninstall --zap portbay` also removes PortBay's local app data, caches, logs, preferences, and WebKit state.

## DMG

Download the latest `PortBay-<version>.dmg` from GitHub Releases, mount it, and drag `PortBay.app` into Applications. The app is signed and notarized for macOS Gatekeeper.

## Build From Source

Source builds are for contributors. The app expects Node, pnpm, Rust, and Tauri prerequisites to be installed.

## Requirements

| Requirement | Notes |
| --- | --- |
| macOS | Signed DMG and Homebrew cask. |
| Linux | AppImage, deb, rpm, Snap, and AUR package targets; requires WebKitGTK, GTK, libayatana-appindicator, libsecret/D-Bus, `dnsmasq`, polkit, and systemd-resolved for full DNS support. |
| Node.js | Use the project’s supported local Node version. |
| pnpm | The repo uses `pnpm-lock.yaml`. |
| Rust | Required for the Tauri core and CLI. |
| Xcode Command Line Tools | Required for native builds on macOS. |

### Clone And Install

```bash
git clone https://github.com/portbay-app/portbay.git
cd portbay
pnpm install
```

### Fetch Development Sidecars

Tauri looks for sidecars under `src-tauri/binaries/<name>-<target-triple>`. Process Compose is committed. The larger or platform-specific tools are fetched per checkout.

```bash
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
./scripts/fetch-dnsmasq.sh
```

The scripts write into the repository checkout. They should be run from the repo root. Do not hand-place global binaries into the app bundle while developing; the app should be able to reproduce its own expected local sidecar layout.

On macOS, `fetch-dnsmasq.sh` bundles the DNS resolver that powers wildcard `*.test` routing, so there is no separate `brew install dnsmasq` step. On Linux, the script writes a small wrapper that calls the system `dnsmasq`; install the distro package before running the app.

### Verify The Checkout

```bash
cd src-tauri
cargo test
cd ..
pnpm check
```

### Run The App

```bash
pnpm tauri dev
```

If the app fails before the first window opens, check that the sidecars are present and executable.
