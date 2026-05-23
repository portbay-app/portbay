# Install

PortBay is currently built from source. The app expects Node, pnpm, Rust, and Tauri prerequisites to be installed.

## Requirements

| Requirement | Notes |
| --- | --- |
| macOS | Primary target for the current implementation. |
| Node.js | Use the project’s supported local Node version. |
| pnpm | The repo uses `pnpm-lock.yaml`. |
| Rust | Required for the Tauri core and CLI. |
| Xcode Command Line Tools | Required for native builds on macOS. |

## Clone And Install

```bash
git clone https://github.com/portbay-app/portbay.git
cd portbay
pnpm install
```

## Fetch Development Sidecars

Tauri looks for sidecars under `src-tauri/binaries/<name>-<target-triple>`. Process Compose is committed. The larger or platform-specific tools are fetched per checkout.

```bash
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
```

The scripts write into the repository checkout. They should be run from the repo root. Do not hand-place global binaries into the app bundle while developing; the app should be able to reproduce its own expected local sidecar layout.

## Verify The Checkout

```bash
cd src-tauri
cargo test
cd ..
pnpm check
```

## Run The App

```bash
pnpm tauri dev
```

If the app fails before the first window opens, check that the sidecars are present and executable.
