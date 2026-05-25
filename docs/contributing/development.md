# Local Development Setup

This guide walks through setting up a local build of PortBay Community from source.
Target platform: **macOS, Apple Silicon**. CI runs on `macos-14` (Apple Silicon) only for now.

---

## Prerequisites

Install these before cloning:

| Tool | How to install |
|---|---|
| Rust (stable toolchain) | `curl https://sh.rustup.rs -sSf \| sh` then `rustup default stable` |
| Xcode Command Line Tools | `xcode-select --install` |
| Node 20 | [nodejs.org](https://nodejs.org/) or `volta install node@20` |
| pnpm 10 | `npm install -g pnpm@10` or `corepack enable && corepack prepare pnpm@10 --activate` |

After installing Rust, confirm:

```bash
rustc --version   # should be stable, 1.77+
cargo --version
```

---

## Clone and install

```bash
git clone https://github.com/portbay-app/portbay.git
cd portbay
pnpm install
```

---

## Fetch sidecar binaries

PortBay bundles several pre-built binaries (Caddy, dnsmasq, mkcert, Process Compose, cloudflared, mailpit). They are **not committed to git** — keeping large platform binaries out of version control avoids repository bloat and allows each developer to fetch the version pinned for their architecture.

Run each fetch script once after cloning (and again when the pinned version changes):

```bash
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
./scripts/fetch-dnsmasq.sh
./scripts/fetch-process-compose.sh
```

Binaries land in `src-tauri/binaries/` which is `.gitignore`d. The `hosts-helper` binary is built from source during the Tauri build step.

---

## Running the app

```bash
pnpm tauri dev
```

This compiles the Rust core, starts the Vite dev server for the frontend, and opens the Tauri window. Expect the first compile to take a few minutes.

---

## Checks, tests, and linting

Run the full check suite before opening a pull request.

### Frontend

```bash
# Type-check Svelte components
pnpm check

# Run unit tests (Vitest)
pnpm test

# Production build (catches bundler errors)
pnpm build
```

### Rust

```bash
cd src-tauri

# Format check
cargo fmt --all -- --check

# Lint (zero warnings allowed)
cargo clippy --all-targets --no-default-features -- -D warnings

# Unit tests
cargo test --no-default-features
```

### Running a subset while iterating

You do not need to run everything on every save. During development:

- Frontend-only changes: `pnpm check && pnpm test`
- Rust-only changes: `cargo clippy` and `cargo test` inside `src-tauri/`
- Full pass before PR: all commands above

---

## CI gate reference

The CI workflow (`.github/workflows/ci.yml`) has three gates:

| Gate | What it runs |
|---|---|
| `rust` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --no-default-features` |
| `frontend` | `pnpm check`, `pnpm test`, `pnpm build` |
| `bundle-smoke` | Debug Tauri build (`pnpm tauri build --debug`) |

All three must pass for a PR to merge.

---

## Docs site

The public docs site is a VitePress project under `docs-site/`.

```bash
# Dev server with hot reload
pnpm docs:dev

# Production build
pnpm docs:build
```

Docs source lives in `docs-site/docs/`. The `docs/` directory in the repo root is for contributor-facing reference documents (what you are reading now) and is separate from the public docs site.

---

## Common issues

**`cargo: command not found` after installing rustup** — restart your terminal or run `source ~/.cargo/env`.

**Sidecar fetch script fails** — check your internet connection and that `curl` is available. The scripts pin specific versions; if upstream changes a download URL, open an issue.

**`pnpm tauri dev` fails with a linker error** — confirm Xcode CLT is installed: `xcode-select -p` should print a path.
