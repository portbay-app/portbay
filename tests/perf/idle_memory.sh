#!/usr/bin/env bash
#
# Idle-RAM budget guard (no WebDriver required).
#
# Launches the packaged PortBay .app against an empty registry, lets it settle,
# then samples the peak resident set size (RSS) of the app process *tree* and
# asserts it stays under the budget. This deliberately avoids the WKWebView
# WebDriver wall (macOS has no embedded-webview WebDriver): we never drive the
# GUI, we just launch the real app and read `ps`.
#
# Usage:
#   tests/perf/idle_memory.sh <path-to-PortBay.app>
#   PORTBAY_APP=/path/to/PortBay.app tests/perf/idle_memory.sh
#
# Tunables (env):
#   IDLE_RAM_MB        budget in MB; 0 = measure-only (report, never fail). Default 80.
#   IDLE_SETTLE_SECS   seconds to wait after launch before sampling.    Default 30.
#   IDLE_SAMPLE_SECS   seconds to sample peak RSS over.                  Default 10.
#
# Exit: 0 pass / measure-only, 1 over budget, 2 usage or launch error.

set -euo pipefail

# Use ONLY system binaries. This machine (and CI) may have ServBay/Homebrew on
# PATH; PortBay never relies on a competitor's toolchain, and neither does its
# tooling. Everything below (find, ps, pgrep, mktemp, plutil, perl) is in /usr.
export PATH="/usr/bin:/bin:/usr/sbin:/sbin"

APP_PATH="${1:-${PORTBAY_APP:-}}"
BUDGET_MB="${IDLE_RAM_MB:-80}"
SETTLE_SECS="${IDLE_SETTLE_SECS:-30}"
SAMPLE_SECS="${IDLE_SAMPLE_SECS:-10}"

if [ -z "$APP_PATH" ]; then
  echo "usage: $0 <path-to-.app>  (or set PORTBAY_APP)" >&2
  exit 2
fi
if [ ! -d "$APP_PATH" ]; then
  echo "error: app bundle not found: $APP_PATH" >&2
  exit 2
fi

# Resolve the REAL main binary via CFBundleExecutable. Contents/MacOS also holds
# PortBay's OWN bundled sidecars (caddy, dnsmasq, process-compose, mkcert, ...),
# so a blind "first executable" would launch the wrong one (it grabbed mkcert).
EXE="$(/usr/bin/plutil -extract CFBundleExecutable raw "$APP_PATH/Contents/Info.plist" 2>/dev/null || true)"
BIN="$APP_PATH/Contents/MacOS/$EXE"
if [ -z "$EXE" ] || [ ! -x "$BIN" ]; then
  echo "error: could not resolve main executable from $APP_PATH/Contents/Info.plist" >&2
  exit 2
fi

# Empty registry: a throwaway HOME so ~/Library/Application Support is pristine
# (no projects, sidecars, or prior state) — this is the "nothing running" idle case.
TMP_HOME="$(mktemp -d)"
APP_PID=""
cleanup() {
  if [ -n "$APP_PID" ]; then
    # kill the whole process group's tree, best-effort.
    pkill -P "$APP_PID" 2>/dev/null || true
    kill "$APP_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_HOME"
}
trap cleanup EXIT

# Recursively collect a pid and all of its descendant pids.
descendants() {
  local parent="$1" child
  for child in $(pgrep -P "$parent" 2>/dev/null); do
    echo "$child"
    descendants "$child"
  done
}

echo "idle_memory: launching $BIN"
echo "idle_memory: HOME=$TMP_HOME  settle=${SETTLE_SECS}s  sample=${SAMPLE_SECS}s  budget=${BUDGET_MB}MB"
HOME="$TMP_HOME" "$BIN" >/dev/null 2>&1 &
APP_PID=$!

# Let the app reach idle steady state.
sleep "$SETTLE_SECS"
if ! kill -0 "$APP_PID" 2>/dev/null; then
  echo "error: app exited before sampling (pid $APP_PID)" >&2
  exit 2
fi

# Sample peak total RSS (KB) of the app + descendants across the sample window.
peak_kb=0
end_ts=$(( $(date +%s) + SAMPLE_SECS ))
while [ "$(date +%s)" -lt "$end_ts" ]; do
  pids="$APP_PID $(descendants "$APP_PID" | tr '\n' ' ')"
  total_kb=0
  for p in $pids; do
    rss="$(ps -o rss= -p "$p" 2>/dev/null | tr -d ' ')"
    if [ -n "$rss" ]; then
      total_kb=$(( total_kb + rss ))
    fi
  done
  if [ "$total_kb" -gt "$peak_kb" ]; then
    peak_kb=$total_kb
  fi
  sleep 1
done

if [ "$peak_kb" -eq 0 ]; then
  echo "error: could not sample RSS for pid $APP_PID" >&2
  exit 2
fi

peak_mb=$(( peak_kb / 1024 ))
echo "idle_memory: peak idle RSS (app + helpers) = ${peak_mb} MB"

if [ "$BUDGET_MB" -eq 0 ]; then
  echo "idle_memory: measure-only (IDLE_RAM_MB=0) — recorded ${peak_mb} MB, not enforced"
  exit 0
fi

if [ "$peak_mb" -gt "$BUDGET_MB" ]; then
  echo "idle_memory: FAIL — ${peak_mb} MB exceeds budget ${BUDGET_MB} MB" >&2
  exit 1
fi

echo "idle_memory: PASS — ${peak_mb} MB within budget ${BUDGET_MB} MB"
exit 0
