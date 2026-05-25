# Architecture

> Short architecture reference for PortBay. Companion to `UX_DESIGN.md`.

---

## High-level diagram

```
┌──────────────────────────────────────────────────────────────┐
│                  PortBay GUI (Tauri 2)                       │
│              Svelte 5 + Tailwind 4 (native atoms)            │
└─────────────────┬────────────────────────────────────────────┘
                  │ Tauri IPC (invoke / events)
┌─────────────────▼────────────────────────────────────────────┐
│              PortBay Core (Rust, single crate)               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  registry/           atomic JSON store + CRUD          │  │
│  │  process_compose/    PC REST client + sidecar lifecycle│  │
│  │  caddy/              Caddy admin client + sidecar      │  │
│  │  mkcert.rs           Cert issuance + lookup            │  │
│  │  hosts.rs            /etc/hosts manager                │  │
│  │  bin/portbay.rs      CLI binary (same core as the GUI) │  │
│  └────────────────────────────────────────────────────────┘  │
└──────┬──────────────┬──────────────┬──────────────┬──────────┘
       │              │              │              │
       ▼              ▼              ▼              ▼
  ┌────────┐    ┌──────────┐    ┌─────────┐    ┌────────┐
  │Process │    │  Caddy   │    │ /etc/   │    │ mkcert │
  │Compose │    │ (admin   │    │ hosts   │    │ (cert  │
  │daemon  │    │  API)    │    │         │    │ issue) │
  └────────┘    └──────────┘    └─────────┘    └────────┘
       │
       ▼
  user project processes (pnpm dev, php-fpm, etc.)
```

---

## Component choices

| Component | Choice | Why |
|---|---|---|
| Desktop shell | **Tauri 2** | <10 MB installer, ~30–50 MB RAM idle. Rust core. Cross-platform from day one. |
| Frontend | **Svelte 5 + Tailwind 4** | Compiler-first reactivity, zero runtime cost, small bundle. Pairs cleanly with Tauri's IPC model. |
| Core language | **Rust 1.95+** | Single binary, no GC pauses during process supervision, Tauri-native. |
| Process daemon | **Process Compose** (Apache 2.0, bundled sidecar) | Mature REST API, health checks, log streaming. Don't reinvent. |
| Reverse proxy | **Caddy 2** (Apache 2.0, bundled sidecar) | Admin API for runtime config, automatic HTTPS, simpler than nginx. |
| Local TLS | **mkcert** (bundled sidecar) | Industry standard. Single dependency. |
| DNS / hostnames | `/etc/hosts` writes (Phase 1) → SMAppService privileged helper (Phase 3) | Hosts file is simple and well-understood; SMAppService removes the per-call sudo prompt. |
| Storage | **JSON file** at `~/Library/Application Support/PortBay/registry.json` | One-user, one-process; JSON is auditable and Git-shareable. |
| Privileged ops | **macOS: SMAppService** | One auth prompt at install, never again. |

---

## Why not just Go?

Considered. Rejected:
- Tauri's GUI story is Rust-native; CGO bridging adds friction.
- Rust + Tauri is where the developer-tools ecosystem is moving (1Password, Fig, Warp post-rewrite).

## Why not Electron?

- 80–150 MB installer vs <30 MB Tauri shell.
- 150–300 MB idle RAM vs 30–50 MB.
- "Lightweight" is the product positioning. Electron immediately invalidates it.
- Tauri's community has crossed the threshold where contributor reach isn't the moat it was in 2023.

---

## Bundle size budget

The `<100 MB` budget is for the **compressed DMG installer** (what a user
downloads), **not** the uncompressed `.app`. The two differ by ~2.3× because
the bundle is dominated by prebuilt Go sidecars that compress to ~⅓ their size.

| Component (release `.app`, measured 2026-05-25, arm64) | Uncompressed |
|---|---|
| `process-compose` sidecar | 42 MB |
| `caddy` sidecar | 38 MB |
| `cloudflared` sidecar (tunnels) | 39 MB |
| `mailpit` sidecar (mail) | 24 MB |
| `portbay-app` (Tauri/Rust shell) | 21 MB&nbsp;* |
| `mkcert` sidecar | 5 MB |
| `portbay` CLI + `portbay-hosts-helper` + `dnsmasq` | ~7 MB |
| Frontend assets, icons, Info.plist | ~1 MB |
| **`.app` total (uncompressed)** | **~175 MB** |
| **DMG installer (UDZO/zlib, measured)** | **~77 MB — under budget** |

\* Before `[profile.release]` stripping (added 2026-05-25: `strip`, `lto`,
`codegen-units = 1`); stripping pulls the shell binary back toward ~12 MB.

The original `<30 MB` target from the planning phase was unreachable once we
accepted bundling all sidecars. The revised `<100 MB` **installer** budget holds
with ~23 MB of headroom. RAM budget (`<80 MB idle`) is unchanged — disk size and
runtime memory are independent. If the installer ever nears 100 MB, the cheapest
lever is lazy-downloading the optional sidecars (cloudflared/tunnels,
mailpit/mail) instead of bundling them.

---

## Where things land on disk

| Path | Purpose |
|---|---|
| `~/Library/Application Support/PortBay/registry.json` | The registry — single source of truth |
| `~/Library/Application Support/PortBay/runtime.json` | Live sidecar port assignments (PC + Caddy admin) |
| `~/Library/Application Support/PortBay/certs/<project-id>/` | mkcert-issued PEMs per project |
| `~/Library/Application Support/PortBay/logs/<project-id>.log` | Per-project PC-managed log file |
| `~/Library/Application Support/PortBay/process-compose.yaml` | Generated PC config (rewritten on every registry mutation) |
| `~/Library/Application Support/PortBay/caddy/autosave.json` | Caddy's own autosave (managed by Caddy itself) |
| `/etc/hosts` | Managed inside a delimited `# BEGIN PortBay` / `# END PortBay` block; everything else strictly untouched |

---

## Phased implementation status

| Phase | Status |
|---|---|
| 0 — Validation spikes (Process Compose, Caddy admin API, Tauri sidecar) | Done |
| 1 — Headless core (registry, adapters, mkcert, hosts, CLI) | Done |
| 2 — GUI MVP (in progress; see kanban) | In progress |
| 3 — UX polish, error handling, signed builds | Planned |
| 4 — Open-source release readiness | Planned |
| 5 — Linux + Windows | Deferred |

See `CHANGELOG.md` and open issues for current focus.
