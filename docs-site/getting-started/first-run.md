---
title: PortBay First Run — Sidecars, Registry & Hostnames
description: "What to verify after launching PortBay for the first time: registry location, sidecar health, dnsmasq wildcard DNS, and the expected data directory layout."
---

# First Run

The first run should establish three things: the registry location, the sidecar health state, and whether PortBay can safely route hostnames on this machine.

<ThemeImage name="services" alt="PortBay services and sidecar health" />

## Expected State

| Area | Expected result |
| --- | --- |
| Registry | Created under the platform app data directory when the first project is saved. |
| Runtime file | Written under the platform app data directory once Process Compose and Caddy have live ports. |
| Sidecars | Process Compose and Caddy should report reachable once started. |
| Hostnames | Project hostnames are routed through Caddy. Exact hostnames resolve via the privileged `/etc/hosts` helper; wildcard `*.test` resolution is handled by dnsmasq. macOS ships PortBay's bundled dnsmasq sidecar; Linux uses the system `dnsmasq` package. |

## What To Check

1. Launch PortBay (the installed app, or `pnpm tauri dev` from a source checkout).
2. Open Settings and confirm the UI theme, density, and sidecar status controls render.
3. Open the Services view and confirm sidecar rows are visible.
4. Open Projects and confirm the empty state renders without errors.

## Data Directory

PortBay stores user data in the platform application support directory. On macOS, the active paths are:

| Path | Purpose |
| --- | --- |
| `~/Library/Application Support/PortBay/registry.json` | Project registry |
| `~/Library/Application Support/PortBay/runtime.json` | Live sidecar port assignments |
| `~/Library/Application Support/PortBay/certs/<project-id>/` | mkcert-issued project certificates |
| `~/Library/Application Support/PortBay/logs/<project-id>.log` | Project logs |
| `~/Library/Application Support/PortBay/process-compose.yaml` | Generated Process Compose config |
| `~/Library/Application Support/PortBay/caddy/autosave.json` | Caddy-managed autosave |

On Linux, the equivalent paths live under `~/.local/share/PortBay/` with the same filenames.
