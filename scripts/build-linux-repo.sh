#!/usr/bin/env bash
# build-linux-repo — assemble a GPG-signed apt + dnf/yum repository from a
# directory of .deb/.rpm packages. The output is a plain static tree you can
# serve from ANY HTTPS host (GitHub Pages, Cloudflare Pages/R2, S3, nginx…), so
# users can `apt install portbay` / `dnf install portbay` with package-manager-
# enforced trust — the standard "trusted install" experience on Linux.
#
# Usage:
#   build-linux-repo.sh <incoming-dir> <output-dir>
#     <incoming-dir>  holds the release's *.deb and *.rpm
#     <output-dir>    the static repo tree to publish (created/overwritten)
#
# Required env:
#   LINUX_GPG_PRIVATE_KEY   armored private key (gpg --export-secret-keys --armor)
# Optional env:
#   LINUX_GPG_PASSPHRASE    the key's passphrase, if it has one
#   REPO_BASE_URL           the public URL the tree will be served at (no trailing
#                           slash) — baked into the generated portbay.repo +
#                           sources.list snippet. Defaults to a placeholder.
#
# Tools (Linux only): gnupg, dpkg-dev (dpkg-scanpackages), apt-utils
# (apt-ftparchive), createrepo_c, gzip. This is Linux-only (the index tools
# don't exist on macOS), so it's exercised in CI, not on the dev Mac.
set -euo pipefail

incoming="${1:?usage: build-linux-repo.sh <incoming-dir> <output-dir>}"
out="${2:?usage: build-linux-repo.sh <incoming-dir> <output-dir>}"
base_url="${REPO_BASE_URL:-https://REPLACE-WITH-YOUR-REPO-HOST}"

: "${LINUX_GPG_PRIVATE_KEY:?LINUX_GPG_PRIVATE_KEY is required (armored private key)}"

# ---- import the signing key -------------------------------------------------
echo "$LINUX_GPG_PRIVATE_KEY" | gpg --batch --import
key_id="$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr:/{print $10; exit}')"
[ -n "$key_id" ] || { echo "build-linux-repo: no secret key imported" >&2; exit 1; }
echo "build-linux-repo: signing with GPG key $key_id"
gpg_args=(--batch --yes --local-user "$key_id")
if [ -n "${LINUX_GPG_PASSPHRASE:-}" ]; then
  gpg_args+=(--pinentry-mode loopback --passphrase "$LINUX_GPG_PASSPHRASE")
fi

rm -rf "$out"
mkdir -p "$out"
# The public key, for users to import + for gpgcheck.
gpg --export --armor "$key_id" > "$out/portbay.asc"

# ---- APT repository ---------------------------------------------------------
# Layout: <out>/apt/{pool/main, dists/stable/main/binary-amd64}. Users add:
#   deb [signed-by=/usr/share/keyrings/portbay.gpg] <base>/apt stable main
apt_root="$out/apt"
pool="$apt_root/pool/main"
comp="$apt_root/dists/stable/main/binary-amd64"
mkdir -p "$pool" "$comp"
if compgen -G "$incoming/*.deb" >/dev/null; then
  cp "$incoming"/*.deb "$pool"/
fi
(
  cd "$apt_root"
  # Filename: paths are relative to the apt root (pool/main/…), matching baseurl.
  dpkg-scanpackages --multiversion pool/main > dists/stable/main/binary-amd64/Packages
)
gzip -9c "$comp/Packages" > "$comp/Packages.gz"
apt-ftparchive \
  -o APT::FTPArchive::Release::Origin="PortBay" \
  -o APT::FTPArchive::Release::Label="PortBay" \
  -o APT::FTPArchive::Release::Suite="stable" \
  -o APT::FTPArchive::Release::Codename="stable" \
  -o APT::FTPArchive::Release::Architectures="amd64" \
  -o APT::FTPArchive::Release::Components="main" \
  release "$apt_root/dists/stable" > "$apt_root/dists/stable/Release"
# Inline-signed (InRelease) + detached (Release.gpg) — apt accepts either.
gpg "${gpg_args[@]}" --clearsign     -o "$apt_root/dists/stable/InRelease"  "$apt_root/dists/stable/Release"
gpg "${gpg_args[@]}" --detach-sign --armor -o "$apt_root/dists/stable/Release.gpg" "$apt_root/dists/stable/Release"

# ---- DNF/YUM repository -----------------------------------------------------
rpm_root="$out/rpm"
mkdir -p "$rpm_root"
if compgen -G "$incoming/*.rpm" >/dev/null; then
  cp "$incoming"/*.rpm "$rpm_root"/
fi
createrepo_c "$rpm_root"
# Sign the repo metadata so `repo_gpgcheck=1` verifies the index itself.
gpg "${gpg_args[@]}" --detach-sign --armor "$rpm_root/repodata/repomd.xml"
cat > "$rpm_root/portbay.repo" <<EOF
[portbay]
name=PortBay
baseurl=${base_url}/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=${base_url}/portbay.asc
EOF

# ---- a tiny landing index + the apt one-liner -------------------------------
cat > "$out/index.html" <<EOF
<!doctype html><meta charset="utf-8"><title>PortBay Linux repository</title>
<h1>PortBay Linux package repository</h1>
<p>Signed apt + dnf repository. See the install instructions in the docs.</p>
<ul>
  <li>Signing key: <a href="portbay.asc">portbay.asc</a> (fingerprint <code>${key_id}</code>)</li>
  <li>apt: <code>${base_url}/apt</code></li>
  <li>dnf: <code>${base_url}/rpm/portbay.repo</code></li>
</ul>
EOF

echo "build-linux-repo: ✓ signed apt+dnf repo under $out (base ${base_url}, key ${key_id})"
