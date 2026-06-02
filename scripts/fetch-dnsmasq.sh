#!/usr/bin/env bash
# fetch-dnsmasq — extract the dnsmasq binary into src-tauri/binaries/
#
# Tauri's sidecar convention expects the binary at
#   src-tauri/binaries/dnsmasq-<rust-target-triple>
# so that PortBay's runtime discovery picks it up before falling back
# to `which dnsmasq`. dnsmasq has no upstream prebuilt macOS release,
# so we source it from Homebrew's pre-compiled bottle — same binary
# the user would otherwise install themselves, just shipped with the
# app so no `brew install dnsmasq` is required on first run.
#
# Re-run after bumping DNSMASQ_VERSION. Idempotent — replaces an
# existing binary if one is in place.

set -euo pipefail

DNSMASQ_VERSION="${DNSMASQ_VERSION:-2.92}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${repo_root}/src-tauri/binaries"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s-$uname_m" in
  Darwin-arm64)   triple="aarch64-apple-darwin"; bottle_tag="arm64_sonoma" ;;
  Darwin-x86_64)  triple="x86_64-apple-darwin";  bottle_tag="sonoma" ;;
  Linux-x86_64)
    triple="x86_64-unknown-linux-gnu"
    mkdir -p "$bin_dir"
    dest="${bin_dir}/dnsmasq-${triple}"
    cat > "$dest" <<'EOF'
#!/usr/bin/env sh
exec dnsmasq "$@"
EOF
    chmod +x "$dest"
    echo "fetch-dnsmasq: Linux uses the system dnsmasq package; wrote wrapper ${dest}"
    exit 0
    ;;
  *)
    echo "fetch-dnsmasq: no bottle mapping for $uname_s-$uname_m; ship from upstream tarball or skip on this host" >&2
    exit 1
    ;;
esac

# Homebrew bottles are hosted on GitHub Container Registry. The
# `oauth2:QQ==` token is the public anonymous token Homebrew itself
# uses — required by ghcr's API even for fully-public images.
bottle_url="https://ghcr.io/v2/homebrew/core/dnsmasq/blobs/sha256:%PLACEHOLDER%"

# The bottle file pattern landed via `brew fetch --bottle-tag=<tag> dnsmasq`,
# producing a tarball at $(brew --cache)/downloads/<digest>--dnsmasq--<ver>.<tag>.bottle.tar.gz.
# Easier and more reliable: drive brew itself, since the user is
# certain to have brew at release time (it's the CI host's package
# manager). The script falls back to the user's existing binary if
# brew isn't on this host.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if command -v brew >/dev/null 2>&1; then
  echo "fetch-dnsmasq: pulling bottle via brew (version ${DNSMASQ_VERSION}, ${bottle_tag})"
  # `brew fetch --bottle-tag` downloads the precompiled tarball
  # without installing it system-wide; quiet output, predictable
  # location in the brew cache.
  # `brew fetch` validates the downloaded bottle against the SHA-256 in the
  # formula before it lands in the cache, so integrity for this path is
  # delegated to Homebrew (a pinned digest here would be fragile — bottles are
  # rebuilt/re-tagged for the same dnsmasq version, unlike immutable upstream
  # release archives).
  brew fetch --bottle-tag="${bottle_tag}" --force "dnsmasq" >/dev/null 2>&1 || \
    brew fetch --force "dnsmasq" >/dev/null 2>&1
  cache_root="$(brew --cache)"
  # Match the exact version + bottle tag so we never pick up an unrelated or
  # stale dnsmasq bottle left in the cache.
  bottle_file="$(find "${cache_root}" -maxdepth 4 -type f -name "*--dnsmasq--${DNSMASQ_VERSION}.${bottle_tag}.bottle.tar.gz" -mmin -5 | head -n1)"
  if [[ -z "${bottle_file}" ]]; then
    # Fall back to a previously downloaded bottle of the SAME version + tag.
    bottle_file="$(ls -t "${cache_root}"/downloads/*--dnsmasq--${DNSMASQ_VERSION}.${bottle_tag}.bottle.tar.gz 2>/dev/null | head -n1 || true)"
  fi
  if [[ -z "${bottle_file}" ]]; then
    echo "fetch-dnsmasq: could not locate downloaded bottle under ${cache_root}" >&2
    exit 1
  fi
  echo "fetch-dnsmasq: extracting ${bottle_file}"
  tar -xzf "${bottle_file}" -C "$tmp"
  # Bottle layout: dnsmasq/<ver>/sbin/dnsmasq
  src_bin="$(find "$tmp/dnsmasq" -type f -name "dnsmasq" -path "*/sbin/*" | head -n1)"
elif [[ -x "/Applications/ServBay/sbin/dnsmasq" ]]; then
  # Last-ditch dev convenience: another local-dev tool ships a
  # compatible dnsmasq. We never bundle this on release builds
  # (CI hosts have brew), but it keeps the dev loop unblocked on
  # machines without brew.
  echo "fetch-dnsmasq: brew unavailable; copying existing dnsmasq from /Applications" >&2
  src_bin="/Applications/ServBay/sbin/dnsmasq"
else
  echo "fetch-dnsmasq: neither brew nor a fallback dnsmasq is available on this host" >&2
  exit 1
fi

if [[ -z "${src_bin}" || ! -f "${src_bin}" ]]; then
  echo "fetch-dnsmasq: could not resolve a dnsmasq binary to extract" >&2
  exit 1
fi

mkdir -p "$bin_dir"
dest="${bin_dir}/dnsmasq-${triple}"
cp "${src_bin}" "$dest"
chmod +x "$dest"

if [[ "$uname_s" == "Darwin" ]]; then
  xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
fi

echo "fetch-dnsmasq: ✓ ${dest}"
"$dest" --version | head -n 1
