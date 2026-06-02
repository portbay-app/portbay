#!/usr/bin/env bash
# Static checks for Linux distribution metadata that should not require a VM or
# package builder.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

node <<'NODE'
const fs = require("fs");
const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
const targets = new Set(conf.bundle?.targets ?? []);
for (const target of ["appimage", "deb", "rpm"]) {
  if (!targets.has(target)) {
    throw new Error(`missing Linux bundle target: ${target}`);
  }
}
for (const dep of ["dnsmasq", "libnotify-bin", "policykit-1", "systemd"]) {
  if (!conf.bundle?.linux?.deb?.depends?.includes(dep)) {
    throw new Error(`missing deb dependency: ${dep}`);
  }
}
for (const dep of ["dnsmasq", "libnotify", "polkit", "systemd"]) {
  if (!conf.bundle?.linux?.rpm?.depends?.includes(dep)) {
    throw new Error(`missing rpm dependency: ${dep}`);
  }
}
NODE

grep -q "confinement: classic" snap/snapcraft.yaml
grep -q "source: portbay.deb" snap/snapcraft.yaml
grep -q "source-type: deb" snap/snapcraft.yaml
grep -q "pkgname=portbay-bin" packaging/aur/PKGBUILD
grep -q "webkit2gtk-4.1" packaging/aur/PKGBUILD
grep -q "REPLACE_WITH_DEB_SHA256" packaging/aur/PKGBUILD
grep -q "REPLACE_WITH_DEB_SHA256" packaging/aur/.SRCINFO

echo "check-linux-packaging: ok"
