#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Point git at the version-controlled hooks in .githooks/ and make them
# executable. Run once after cloning. The `prepare` npm script runs this
# automatically on `pnpm install`, so most contributors never run it by hand.

set -euo pipefail

if ! git rev-parse --show-toplevel >/dev/null 2>&1; then
  echo "not a git repository — skipping hook setup" >&2
  exit 0
fi

cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/* 2>/dev/null || true

echo "PortBay git hooks installed (core.hooksPath=.githooks)."
echo "  pre-commit: blocks Cloud/Pro code, secrets, and forbidden files (staged)."
echo "  pre-push:   full-tree boundary + secret scan before reaching the remote."
echo "For local secret scanning, also: brew install gitleaks"
