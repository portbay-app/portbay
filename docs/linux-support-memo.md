# Linux Support Implementation Memo

Date: 2026-05-31; refreshed 2026-06-02

## Phase 0 Decision

Go, with Ubuntu 22.04/24.04 and Debian 12 as the primary Linux desktop build
baseline. Fedora/RHEL, Arch Linux/Manjaro, AppImage, Snap, deb, rpm, and AUR
are now explicit package targets; each still needs VM QA evidence before the
production release is marked final.

## Chosen DNS Mechanism

PortBay's first Linux wildcard DNS implementation targets `systemd-resolved`.
The privileged helper writes a managed drop-in under:

```text
/etc/systemd/resolved.conf.d/portbay-<suffix>.conf
```

The drop-in routes `~<suffix>` queries to PortBay's loopback `dnsmasq` listener
on the current high port. Exact project hostnames remain covered by the
`/etc/hosts` helper path, so project startup does not depend solely on wildcard
DNS.

NetworkManager-only and resolvconf-only environments are deferred. The app and
docs report them as best-effort rather than pretending there is one portable
Linux resolver API.

## Sidecar Strategy

- `caddy`, `mailpit`, `process-compose`, `cloudflared`, and `mkcert`: bundle
  upstream Linux x86_64 binaries with pinned SHA-256 verification.
- `dnsmasq`: require the system package for Linux v1. The fetch script writes a
  wrapper sidecar that executes `dnsmasq` from PATH so Tauri's externalBin
  contract is still satisfied.
- `portbay-hosts-helper` and `portbay-mcp`: built from source for the host
  target by the existing scripts.

## Distribution Strategy

- **AppImage:** universal x86_64 artifact and the Tauri updater target.
- **deb:** first-class Debian/Ubuntu package emitted by Tauri with PortBay's
  runtime dependencies declared.
- **rpm:** first-class Fedora/RHEL-compatible package emitted by Tauri with
  PortBay's runtime dependencies declared.
- **Snap:** built from the release deb with Snapcraft's `dump` plugin. The snap
  uses classic confinement because PortBay needs host DNS, polkit, systemd,
  `/etc/hosts`, process, and Secret Service integration.
- **AUR:** `portbay-bin` extracts the release deb, matching Tauri's recommended
  AUR flow for prebuilt desktop packages. Release automation can publish it
  when `AUR_PUBLISH_ENABLED` and `AUR_SSH_PRIVATE_KEY` are configured.

## Privilege Path Security Review

Linux helper installation uses `pkexec` for the one privileged install step,
then registers a root-owned systemd service:

```text
/etc/systemd/system/portbay-hosts-helper.service
```

The installed daemon listens on `/var/run/portbay-hosts-helper.sock`, owns the
socket as the installing UID at `0600`, and verifies every accepted connection
with Linux `SO_PEERCRED`. Requests are still constrained by PortBay's suffix
validation before any `/etc/hosts` or resolver mutation is attempted.

The polkit policy is installed to:

```text
/usr/share/polkit-1/actions/com.portbay-app.portbay.hosts-helper.install.policy
```

The current implementation still shells the install script through `pkexec`.
Before a production release, this path should be reviewed in a real Ubuntu VM
and, if needed, tightened into a dedicated installer subcommand so polkit can
authorize a narrower executable than `/bin/sh`.

## Verified Compile Tail

Local macOS build:

```text
cargo check --no-default-features
```

Result: passed.

Linux target from macOS:

```text
cargo check --target x86_64-unknown-linux-gnu --no-default-features
```

Result: blocked before PortBay code compilation by GTK/WebKit native
dependencies. `pkg-config` reported that cross-compilation was not configured
for `glib-sys`, `gobject-sys`, `gio-sys`, and `gdk-sys`. This is an environment
limitation of attempting a Linux Tauri build from macOS, not evidence of a
PortBay Linux code error. The added Ubuntu CI job is the authoritative compile
tail for Linux.

## QA Matrix Status

| Environment | Status | Evidence |
| --- | --- | --- |
| macOS local Rust check | Passed | 2026-06-02: `cargo check --no-default-features`, `cargo clippy --all-targets --no-default-features -- -D warnings` |
| macOS local Rust tests | Passed | 2026-06-02: `cargo test --no-default-features` before the sync-key fallback patch; post-patch `cargo check` and clippy passed |
| Frontend/unit checks | Passed | 2026-06-02: `pnpm check`, `pnpm test -- --run` before the sync-key fallback patch; post-patch docs-only frontend change passed `pnpm docs:build` |
| Packaging metadata | Passed | 2026-06-02: `scripts/check-linux-packaging.sh`; validates AppImage/deb/rpm targets plus Snap/AUR metadata |
| macOS local Tauri debug build | Passed | 2026-06-02: `pnpm tauri build --debug --no-bundle` |
| macOS to Linux cross-check | Blocked | Missing Linux sysroot/pkg-config setup for GTK stack |
| Ubuntu 22.04 CI | Pending | `.github/workflows/ci.yml` adds `rust-linux`, `bundle-smoke-linux`, and package metadata checks; branch was not pushed to `origin` on 2026-06-02, so no Actions run exists |
| Ubuntu 24.04 VM | Pending | Needs deb/AppImage/Snap run |
| Debian 12 VM | Pending | Needs deb/AppImage/Snap run |
| Fedora VM | Pending | Needs rpm/AppImage run |
| RHEL-compatible VM | Pending | Needs rpm/AppImage run |
| Arch Linux / Manjaro VM | Pending | Needs AUR/AppImage run |
| X11 | Pending | Needs AppImage/deb smoke |
| Wayland | Pending | Needs AppImage/deb smoke |

## Known Limitations

- Sandboxed Run is disabled on Linux for the first release. The app refuses to
  run "sandboxed" without a Linux sandbox backend.
- Wildcard DNS supports `systemd-resolved` first. Other resolver managers are
  best-effort.
- Secret Service absence falls back to `PORTBAY_SESSION_JSON` or a local
  `~/.config/PortBay/session.json` file with `0600` permissions for account
  sessions, and to `PORTBAY_SYNC_KEY` or `~/.config/PortBay/sync.key` with
  `0600` permissions for sync recovery keys.
- AppImage uses the in-app updater; deb, rpm, Snap, and AUR packages should
  update through their package managers.
- Wayland transparency and focus behavior vary by compositor.
- X11 single-instance focus can require `xdotool`; Wayland may prevent focus
  stealing.

## Bundle-Size Delta

Pending. Requires successful Linux AppImage/deb/rpm/Snap build on Ubuntu and
comparison against the current macOS release artifact.
