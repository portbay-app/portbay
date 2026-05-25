# Architecture Orientation for Contributors

This document is a contributor-facing introduction to how PortBay is structured.
The full architecture reference is at [docs/ARCHITECTURE.md](../../ARCHITECTURE.md) (repo root).

---

## High-level model

```
GUI (Tauri window)
  Svelte 5 + SvelteKit frontend
      │
      │  Tauri IPC (invoke / events)
      ▼
Rust core  (src-tauri/)
  ├── Commands  (src-tauri/src/commands/)
  ├── Daemon / reconciler
  ├── Declarative project registry  (JSON on disk)
  └── Sidecar management
        │
        ├── Process Compose  — process supervision
        ├── Caddy             — reverse proxy + admin API
        ├── dnsmasq           — wildcard *.test DNS
        ├── mkcert            — local HTTPS certificates
        ├── hosts-helper      — privileged /etc/hosts writes (built from source)
        ├── cloudflared       — outbound tunnels
        └── mailpit           — local mail capture
```

The **Rust core is the source of truth**. The GUI is a client of the Rust core, not the other way around. Every operation the GUI can perform is also available via the CLI, keeping full CLI parity.

---

## Declarative registry

Projects are stored as JSON in a declarative registry on the user's machine. A daemon process reads the registry and reconciles running state — starting or stopping sidecars and services as needed. Contributors should read and write the registry through the existing Rust registry module rather than manipulating files directly.

---

## Where code lives

| Path | What lives there |
|---|---|
| `src/` | Svelte 5 + SvelteKit frontend |
| `src-tauri/src/` | Rust core, library modules |
| `src-tauri/src/commands/` | Tauri IPC command handlers — all `tauri::command` functions go here |
| `src-tauri/src/main.rs` | App entry point; registers commands |
| `src-tauri/binaries/` | Fetched sidecar binaries (not committed) |
| `scripts/` | Fetch scripts for sidecars and CI tooling |
| `docs/` | Contributor reference documents (this tree) |
| `docs-site/` | VitePress public documentation site |

---

## Rust conventions

- Errors: use `AppError` (the repo's structured error type). Do not return raw `String` errors from commands.
- Commands: IPC command functions live in `src-tauri/src/commands/` and are registered in `main.rs`. Keep them thin — delegate logic to library modules.
- Filesystem writes that touch user data (project paths, registry, environment) must be atomic.
- `cargo clippy --all-targets --no-default-features -- -D warnings` must pass with zero warnings.

---

## Frontend conventions

- State lives in Svelte stores. Prefer existing stores over new global state.
- IPC calls go through the Tauri `invoke` helper; wrap them in typed functions rather than calling `invoke` directly in components.
- `pnpm check` (svelte-check) must pass with zero errors.

---

## The cloud-client boundary

PortBay Community may contain **client-side code** that talks to PortBay Cloud public APIs — for example, verifying a signed entitlement token or initiating a tunnel connection. This code must:

- Call only documented, public cloud API endpoints (no internal endpoints)
- Verify signatures using a **public key** embedded in the app (never a private key)
- Live behind a thin, generic interface so the cloud dependency is explicit and auditable
- Never embed secrets, private keys, or customer data

The server-side implementation of those APIs lives exclusively in `portbay-app/portbay-cloud`. See [docs/architecture/repo-boundaries.md](../architecture/repo-boundaries.md) and [license-policy.md](license-policy.md) for the full boundary specification and the CI guard that enforces it.

---

## Adding a new sidecar

New bundled binaries require a discussion first (see [overview.md](overview.md)). The process:

1. Open a GitHub Discussion explaining the use case and why the sidecar cannot be replaced by an existing one.
2. If accepted, add a fetch script under `scripts/fetch-<name>.sh` following the pattern of existing scripts.
3. Add the sidecar to the Tauri `externalBin` list in `src-tauri/tauri.conf.json`.
4. Add Rust management code following the pattern of existing sidecar modules.
5. Update `development.md` to include the new fetch step.

---

## Adding a Tauri capability

New `tauri:allow-*` permissions require justification in the PR body. Maintainers look for minimal scope — request only what the feature actually needs, not a broader permission that is convenient. See [pull-requests.md](pull-requests.md).
