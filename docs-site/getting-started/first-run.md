---
title: PortBay First Run — Sidecars, Registry & Hostnames
description: "What to verify after launching PortBay for the first time: registry location, sidecar health, dnsmasq wildcard DNS, and the expected data directory layout."
---

# First Run

The first run should establish three things: the registry location, the sidecar health state, and whether PortBay can safely route hostnames on this machine.

<ThemeImage name="services" alt="PortBay services and sidecar health" />

## System Authorization Prompts

When you add your first project, macOS will present two authorization dialogs in sequence. PortBay shows an in-app explainer screen before this happens so you know what to expect.

### Prompt 1 — Keychain CA trust

**What it is:** macOS asking you to allow a new root certificate into your login Keychain.

**Why it appears:** PortBay runs `mkcert -install` to create a local certificate authority (CA) and register it as trusted. After this, every `.test` certificate PortBay issues is trusted by Safari, Chrome, and Firefox without a browser warning.

**What it touches:**

- Creates `rootCA.pem` under `~/Library/Application Support/mkcert/` (the path `mkcert -CAROOT` prints).
- Adds that certificate to your macOS login Keychain with the "Always Trust" policy for SSL.

**What it does not do:** This CA is not a public internet CA. It cannot sign certificates for any domain you do not control locally, and it has no authority outside your machine.

**If you decline:** PortBay continues to work, but browsers will show a certificate warning for every `.test` project that has HTTPS enabled. To install the CA later, go to **Settings → Domains & HTTPS** and click **Install CA** next to the "Trust local CA" row — this fires the same Keychain prompt on demand. Alternatively, run `portbay hosts reconcile` from the CLI.

---

### Prompt 2 — Admin password (privileged helper install)

**What it is:** A standard macOS system dialog asking for your administrator password.

**Why it appears:** PortBay installs a small privileged helper binary at `/usr/local/bin/portbay-hosts-helper` and registers it as a macOS LaunchDaemon (`com.portbay-app.portbay.hosts-helper`). The helper runs as root because editing `/etc/hosts` and writing resolver files under `/etc/resolver/` require elevated privilege.

**The helper's complete scope:**

| Can do | Cannot do |
| --- | --- |
| Write project hostname entries into `/etc/hosts` — strictly limited to hostnames ending in your configured `.test` suffix | Modify any hostname outside your `.test` suffix; the suffix guard is enforced before any write |
| Write `/etc/resolver/<suffix>` so macOS routes `*.test` queries to PortBay's local dnsmasq resolver | Accept connections from any other local user — the Unix socket at `/var/run/portbay-hosts-helper.sock` is locked to your user ID at the OS level |
| Remove the resolver file or clear PortBay-managed hosts entries when you uninstall or change your domain suffix | Proxy network traffic, read project files, or access any other part of the filesystem |

The helper is not persistent beyond the two operations above. It listens on the Unix socket, applies a request, and returns. The LaunchDaemon keeps it alive so PortBay can call it without a new authorization prompt each time.

**If you decline:** Project hostnames will not resolve in your browser (the hosts file is not updated), and `*.test` wildcard DNS will not work. PortBay itself stays functional — the project list, sidecar management, and the scaffolder all work. You can install the helper later by going to **DNS** in the sidebar and clicking "Set up local DNS." As a manual fallback, `sudo portbay hosts reconcile` from a terminal writes the hosts entries without the daemon.

---

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
