# PortBay

> Lightweight, open-source local development environment manager.
> One Play button per project. One universal Stop.

**Status: pre-MVP, in active development. Not ready for general use.**

PortBay is a small native app for macOS (Linux + Windows later) that replaces the parts of ServBay, MAMP, and Laravel Herd that developers actually use:

- One-click Play and Stop per project (Next.js, Vite, PHP, Laravel, plain Node).
- A universal Stop-All kill switch that always works.
- Local HTTPS hostnames like `https://myproject.test`.
- Reverse-proxy routing via [Caddy](https://caddyserver.com).
- A registry-driven model — declare projects once in JSON; the daemon does the rest.

## Why another one?

ServBay is closed source with a paywall on essentials. Laravel Herd is PHP-only. FlyEnv and Lerd are good but heavy (Electron / container-based). PortBay sits in the gap: native, lightweight, project-launcher-first, and welcoming to both beginners using AI coding tools and senior engineers who want the CLI to be a first-class citizen.

Target footprint: **under 80 MB idle RAM, installer under 30 MB.** Built with Tauri 2 + Rust + Svelte, with Process Compose handling lifecycle and Caddy handling routes.

## Architecture (planned)

```
GUI (Tauri + Svelte)
  └─ HTTP → PortBay Core (Rust)
              ├─ Process Compose  (daemon — manages your dev processes)
              ├─ Caddy            (reverse proxy — admin API for runtime routes)
              ├─ mkcert           (local HTTPS certs)
              └─ Hosts file       (privileged helper)
```

Full assessment, architecture, and phased implementation plan live in [`docs/ASSESSMENT_AND_PLAN.md`](./docs/ASSESSMENT_AND_PLAN.md) once we land that doc here. (Currently in the planning workspace; will be ported into this repo before Phase 1.)

## What's not here yet

- Any working code. Repo is the project home; first spike code lands when Phase 0 validation completes.
- A landing page. We're GitHub-only for now.
- Distribution channels (no Homebrew tap, no npm, no notarized builds). All of these come later — only when there's demand.

## Roadmap

Tracked publicly via GitHub Projects (link added once enabled). High-level phases:

1. **Phase 0** — validation spikes (Process Compose, Caddy admin API, Tauri sidecar).
2. **Phase 1** — headless Rust core with CLI parity.
3. **Phase 2** — Tauri GUI MVP.
4. **Phase 3** — UX polish, error handling, onboarding.
5. **Phase 4** — open-source release readiness.

## Development setup

PortBay bundles two third-party sidecars — `process-compose` and `caddy` —
that Tauri expects to find under `src-tauri/binaries/<name>-<target-triple>`.
Process Compose is committed to the repo; Caddy is fetched per checkout
because the binary is large and platform-specific:

```bash
./scripts/fetch-caddy.sh     # writes src-tauri/binaries/caddy-<triple>
./scripts/fetch-mkcert.sh    # writes src-tauri/binaries/mkcert-<triple>
./scripts/fetch-mailpit.sh   # writes src-tauri/binaries/mailpit-<triple>
```

Re-run after bumping the version constant inside any script. On a fresh
clone the dev server (`pnpm tauri dev`) will fail to start until these
binaries are in place. The dnsmasq sidecar is currently resolved from
PATH; an equivalent `fetch-dnsmasq.sh` will land once the resolver-file
install flow needs production bundling.

## Contributing

Not open to contributions yet — the project is in early validation. Once Phase 1 lands, `CONTRIBUTING.md` will go up. Star the repo if you want to be notified.

## License

[MIT](./LICENSE).
