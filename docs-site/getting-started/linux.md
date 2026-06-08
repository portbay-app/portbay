---
title: Linux Support
description: Linux desktop support status, package requirements, DNS strategy, and known limitations for PortBay.
---

# Linux Support

PortBay's Linux desktop support covers AppImage, deb, rpm, Snap, and AUR-based
installs. Ubuntu 22.04/24.04 and Debian 12 are the primary build baseline;
Fedora/RHEL and Arch/Manjaro are supported package targets once their QA rows
are green.

## Package Formats

| Format | Distros | Status |
| --- | --- | --- |
| AppImage | Most x86_64 desktop distros | Universal release artifact and updater target. |
| deb | Debian / Ubuntu | Native package with runtime dependencies declared. |
| rpm | Fedora / RHEL-compatible distros | Native package with runtime dependencies declared. |
| Snap | Ubuntu, Debian, and Snap-enabled distros | Classic-confinement package built from the release deb. |
| AUR | Arch Linux / Manjaro | `portbay-bin` package that extracts the release deb. |

## Build Dependencies

Install the desktop and runtime dependencies before building from source on
Debian or Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libdbus-1-dev \
  libsecret-1-dev \
  dnsmasq \
  libnotify-bin \
  policykit-1 \
  rpm
```

On Fedora/RHEL-family systems, install the equivalent packages:

```bash
sudo dnf install -y \
  webkit2gtk4.1-devel \
  gtk3-devel \
  libayatana-appindicator-gtk3-devel \
  librsvg2-devel \
  dbus-devel \
  libsecret-devel \
  dnsmasq \
  libnotify \
  polkit \
  rpm-build
```

On Arch Linux / Manjaro:

```bash
sudo pacman -S --needed \
  webkit2gtk-4.1 \
  gtk3 \
  libayatana-appindicator \
  librsvg \
  dbus \
  libsecret \
  dnsmasq \
  libnotify \
  polkit
```

Release deb/rpm/AUR packages declare the runtime equivalents of these
dependencies. AppImage builds still expect `dnsmasq`, polkit, and a working
`systemd-resolved` setup on the host. The Snap package uses classic confinement
because PortBay needs host DNS, helper, process, and Secret Service integration.

## DNS And Hosts

Linux wildcard DNS uses `systemd-resolved` in the first supported tier. PortBay writes a managed drop-in under:

```text
/etc/systemd/resolved.conf.d/portbay-<suffix>.conf
```

The drop-in points `~<suffix>` queries at PortBay's loopback `dnsmasq` listener. Exact project hostnames still use the privileged hosts helper as the guaranteed fallback.

## Privileged Helper

The Linux helper is installed with a polkit prompt and registered as a systemd service. It listens on the same Unix socket as macOS:

```text
/var/run/portbay-hosts-helper.sock
```

The helper checks the connecting process UID with Linux `SO_PEERCRED` and only accepts the installing user or root.

## Known Limitations

- Sandboxed Run is disabled on Linux for the first release. PortBay refuses to run a project "sandboxed" without a Linux sandbox backend.
- Wildcard DNS currently supports `systemd-resolved`; NetworkManager-only and resolvconf-only setups are best-effort.
- AppImage auto-update is supported by the Tauri updater channel. deb, rpm,
  Snap, and AUR updates are expected to flow through their package managers.
- Wayland compositors differ in transparency and tray behavior. PortBay uses an opaque Linux shell fallback instead of macOS vibrancy.
- If Secret Service is missing, auth falls back to `PORTBAY_SESSION_JSON` or a local `~/.config/PortBay/session.json` file with `0600` permissions; sync recovery keys fall back to `PORTBAY_SYNC_KEY` or `~/.config/PortBay/sync.key` with `0600` permissions. Treat both environment overrides as security-sensitive — anything that can read the process environment can read the tokens (see the "Session Environment Override" section of [SECURITY.md](https://github.com/portbay-app/portbay/blob/main/SECURITY.md)); the app logs a warning whenever a session is loaded from the environment.
- The single-instance plugin may require `xdotool` on X11; Wayland focus-stealing restrictions can prevent second-launch focus.

## Local Build

```bash
./scripts/fetch-process-compose.sh
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
./scripts/fetch-dnsmasq.sh
./scripts/build-hosts-helper.sh
./scripts/build-mcp.sh
pnpm tauri build --debug --no-bundle
```

For a local AppImage smoke build, run:

```bash
./scripts/release-appimage-local.sh
```

For local AppImage + deb + rpm smoke artifacts on Linux x86_64, run:

```bash
./scripts/release-linux-local.sh
```
