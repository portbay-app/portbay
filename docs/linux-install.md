# Installing PortBay on Linux

PortBay ships for Linux x86_64 as an **AppImage** (portable, auto-updating), a
**.deb** and **.rpm** (system install), and via a signed **apt/dnf repository**.

> Replace `https://portbay-app.github.io/portbay` below with your repository URL
> if you publish it on a custom domain (set the `LINUX_REPO_BASE_URL` repo
> variable + a Pages custom domain).

## Option 1 — AppImage (no install, auto-updates)

```bash
chmod +x PortBay-linux-x86_64.AppImage
./PortBay-linux-x86_64.AppImage
```

Optionally verify the download against PortBay's GPG key first (if GPG signing is
enabled — the `.asc` is published next to the AppImage):

```bash
curl -fsSLO https://portbay-app.github.io/portbay/portbay.asc
gpg --import portbay.asc
gpg --verify PortBay-linux-x86_64.AppImage.asc PortBay-linux-x86_64.AppImage
```

> `.test` hostnames need **systemd-resolved**. On Debian/Ubuntu/Fedora it's the
> default; on other distros enable it or PortBay falls back to a degraded DNS tier.

## Option 2 — Debian / Ubuntu (apt repository, recommended)

```bash
# Trust PortBay's signing key (stored as a keyring, not apt-key).
curl -fsSL https://portbay-app.github.io/portbay/portbay.asc \
  | sudo gpg --dearmor -o /usr/share/keyrings/portbay.gpg

# Add the repository.
echo "deb [signed-by=/usr/share/keyrings/portbay.gpg] https://portbay-app.github.io/portbay/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/portbay.list

sudo apt update
sudo apt install portbay
```

Updates then arrive through `apt upgrade` like any other package.

## Option 3 — Fedora / RHEL / openSUSE (dnf/yum repository)

```bash
sudo curl -fsSL https://portbay-app.github.io/portbay/rpm/portbay.repo \
  -o /etc/yum.repos.d/portbay.repo
sudo dnf install portbay
```

The `.repo` file pins PortBay's GPG key, so `dnf` verifies both the package and
the repository metadata (`gpgcheck=1`, `repo_gpgcheck=1`).

## Option 4 — direct .deb / .rpm

```bash
sudo apt install ./PortBay_0.1.6_amd64.deb        # Debian/Ubuntu
sudo dnf install ./PortBay-0.1.6-1.x86_64.rpm     # Fedora/RHEL
```

## Option 5 — Snap (if published)

```bash
sudo snap install portbay --classic
```

(Classic confinement — PortBay manages host networking/DNS, which the snap
sandbox can't contain.)

---

### A note on "signing" and trust on Linux

There is no Gatekeeper/SmartScreen on Linux — an AppImage just runs, signed or
not. The signatures here are about **provenance and integrity**: the minisign
signature secures **auto-update**, the GPG signatures let you **verify
downloads**, and the apt/dnf repository gives the package manager **enforced
trust** on install + upgrade. PortBay is *not* distributed via Flatpak/Flathub —
its sandbox is incompatible with managing host DNS, `/etc/hosts`, and privileged
ports.
</content>
