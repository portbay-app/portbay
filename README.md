# PortBay

> Lightweight, open-source local development environment manager.
> One Play button per project. One universal Stop.

**Status: pre-MVP, in active development. Not ready for general use.**

PortBay is a small native app for macOS (Linux + Windows later) that gives developers a fast, predictable way to run multiple local projects side by side — without the weight of a container stack or the friction of hand-rolling proxy, DNS, and TLS configs for every project.

- One-click Play and Stop per project (Next.js, Vite, PHP, Laravel, plain Node).
- A universal Stop-All kill switch that always works.
- Local HTTPS hostnames like `https://myproject.test`.
- Reverse-proxy routing via [Caddy](https://caddyserver.com).
- A registry-driven model — declare projects once in JSON; the daemon does the rest.

## The problem we're solving

Modern local development is fragmented. Engineers juggle background processes (`pnpm dev`, `php-fpm`, `redis-server`, `vite`), competing ports, ad-hoc `/etc/hosts` edits, self-signed certs, and a reverse proxy or two — multiplied across every project they own. The result is forgotten background processes, port conflicts, expired certs, and "it worked yesterday" mornings.

PortBay treats your machine like a tiny PaaS: each project is a declarative record with a hostname, a start command, and a port. The app handles the lifecycle, the routing, and the certificates. Stopping the app stops every project. Restarting one project doesn't disturb the others.

The design constraint is to stay **native and small** — under 80 MB idle RAM, installer under 30 MB — so it fits alongside an editor, a browser, and a chat client without being noticed.

Built with Tauri 2 + Rust + Svelte, with Process Compose handling lifecycle and Caddy handling routes.

## Architecture (planned)

```
GUI (Tauri + Svelte)
  └─ HTTP → PortBay Core (Rust)
              ├─ Process Compose  (daemon — manages your dev processes)
              ├─ Caddy            (reverse proxy — admin API for runtime routes)
              ├─ mkcert           (local HTTPS certs)
              └─ Hosts file       (privileged helper)
```

Full architecture lives in [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md); UX principles in [`docs/UX_DESIGN.md`](./docs/UX_DESIGN.md).

## What's not here yet

- Distribution channels (no Homebrew tap, no notarized builds). These come once there's demand.
- A landing page. GitHub-only for now.
- General-availability stability. Phase 3 polish is in flight.

## Roadmap

High-level phases:

1. **Phase 0** — validation spikes (Process Compose, Caddy admin API, Tauri sidecar).
2. **Phase 1** — headless Rust core with CLI parity.
3. **Phase 2** — Tauri GUI MVP.
4. **Phase 3** — UX polish, error handling, onboarding.
5. **Phase 4** — open-source release readiness.

## Development setup

PortBay bundles third-party sidecars — `process-compose`, `caddy`, `mkcert`,
`mailpit`, and `cloudflared` — that Tauri expects to find under
`src-tauri/binaries/<name>-<target-triple>`. Process Compose is committed
to the repo; the others are fetched per checkout because the binaries are
large and platform-specific:

```bash
./scripts/fetch-caddy.sh        # writes src-tauri/binaries/caddy-<triple>
./scripts/fetch-mkcert.sh       # writes src-tauri/binaries/mkcert-<triple>
./scripts/fetch-mailpit.sh      # writes src-tauri/binaries/mailpit-<triple>
./scripts/fetch-cloudflared.sh  # writes src-tauri/binaries/cloudflared-<triple>
```

Re-run after bumping the version constant inside any script. On a fresh
clone the dev server (`pnpm tauri dev`) will fail to start until these
binaries are in place. The `dnsmasq` sidecar is currently resolved from
PATH; an equivalent fetch script will land once the resolver-file install
flow needs production bundling.

## Contributing

Not open to contributions yet — the project is in early validation. Once Phase 1 lands publicly, `CONTRIBUTING.md` will go up. Star the repo if you want to be notified.

## License

[MIT](./LICENSE).
