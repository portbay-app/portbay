#!/bin/bash
# Offline probe for the Smart Dictation prompt against the portbay-afm
# sidecar — the manual gate behind the "re-probe after ANY prompt edit" rule
# (the prompt in src-tauri/src/dictation.rs is load-bearing on the 3B: tiny
# wording changes flip behaviors; see README.md).
#
# Usage:
#   probe.sh <system-prompt-file> <transcript-file> [maxTokens]
#   probe.sh --raw <system-prompt-file> <transcript-file> [maxTokens]
#
# Default frames the user message exactly as production does
# ("Transcript: <text>" — see dictation.rs::build_user; the framing is
# load-bearing too). --raw sends the transcript unframed, for experiments.
#
# Needs Apple Intelligence hardware (macOS 26+, eligible device) — NOT wired
# into CI on purpose.
set -euo pipefail

framed=1
if [ "${1:-}" = "--raw" ]; then
  framed=0
  shift
fi
if [ $# -lt 2 ]; then
  echo "usage: probe.sh [--raw] <system-prompt-file> <transcript-file> [maxTokens]" >&2
  exit 64
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
arch="$(uname -m)"
case "$arch" in
  arm64) triple="aarch64-apple-darwin" ;;
  x86_64) triple="x86_64-apple-darwin" ;;
  *) echo "probe.sh: unsupported arch $arch" >&2; exit 1 ;;
esac
AFM="${repo_root}/src-tauri/binaries/portbay-afm-${triple}"
if [ ! -x "$AFM" ]; then
  echo "probe.sh: sidecar missing — run scripts/build-afm.sh first" >&2
  exit 1
fi

SYSTEM=$(cat "$1")
if [ "$framed" = 1 ]; then
  PROMPT="Transcript: $(cat "$2")"
else
  PROMPT=$(cat "$2")
fi
MAX=${3:-800}

jq -n --arg system "$SYSTEM" --arg prompt "$PROMPT" --argjson max "$MAX" \
  '{system:$system, prompt:$prompt, maxTokens:$max}' | "$AFM"
echo ""
echo "--- exit=$? in_chars=$(printf %s "$PROMPT" | wc -c | tr -d ' ')"
