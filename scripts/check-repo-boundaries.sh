#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# check-repo-boundaries.sh
#
# Fails if the public portbay-app repository contains anything that belongs only
# in the private portbay-cloud repository: proprietary Cloud/Pro code, secret
# values, private endpoints, or files that must never be committed.
#
# Rules live in `.repo-boundary-denylist`. Known-legitimate files (docs that
# quote the forbidden terms, the committed `.env.example`, etc.) are skipped via
# `.repo-boundary-allow`.
#
# Modes:
#   (default)   Scan tracked + untracked-not-ignored files. Used by CI and
#               `pre-push`. Catches a leak anywhere in the working tree.
#   --staged    Scan only what is staged for commit (the exact blobs that would
#               enter history). Used by the `pre-commit` hook so a bad commit is
#               blocked BEFORE it is created — no force-clean, ever.
#
# Exit: 0 = clean   1 = boundary violation(s)   2 = misconfiguration
#
# Written for bash 3.2 (macOS / GitHub-runner default). Assumes tracked/staged
# paths contain no spaces; it errors clearly if that ever stops holding.

set -euo pipefail

MODE="full"
case "${1:-}" in
  --staged) MODE="staged" ;;
  "" ) ;;
  * ) echo "usage: $0 [--staged]" >&2; exit 2 ;;
esac

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT"

DENYLIST="${REPO_BOUNDARY_DENYLIST:-.repo-boundary-denylist}"
ALLOWFILE="${REPO_BOUNDARY_ALLOW:-.repo-boundary-allow}"
SELF="scripts/check-repo-boundaries.sh"

if [ ! -f "$DENYLIST" ]; then
  echo "error: denylist not found: $DENYLIST" >&2
  exit 2
fi

tmp_files="$(mktemp)"
trap 'rm -f "$tmp_files"' EXIT

# ---- Build the scan list -----------------------------------------------------
if [ "$MODE" = "staged" ]; then
  # Added/copied/modified/renamed entries in the index.
  raw_list() { git diff --cached --name-only --diff-filter=ACMR; }
  # Read the STAGED blob (not the working tree) so partial `git add -p` is exact.
  read_file() { git show ":$1" 2>/dev/null || true; }
else
  raw_list() { git ls-files --cached --others --exclude-standard; }
  read_file() { cat "$1" 2>/dev/null || true; }
fi

raw_list | while IFS= read -r f; do
  [ -z "$f" ] && continue
  case "$f" in
    "$DENYLIST"|"$ALLOWFILE"|"$SELF") continue ;;
  esac
  case "$f" in *" "*)
    echo "error: tracked/staged path contains a space, unsupported: $f" >&2
    exit 2 ;;
  esac
  if [ -f "$ALLOWFILE" ]; then
    skip=0
    while IFS= read -r glob; do
      [ -z "$glob" ] && continue
      case "$glob" in \#*) continue ;; esac
      # shellcheck disable=SC2254
      case "$f" in $glob) skip=1; break ;; esac
    done < "$ALLOWFILE"
    [ "$skip" = "1" ] && continue
  fi
  printf '%s\n' "$f"
done > "$tmp_files"

scanned="$(wc -l < "$tmp_files" | tr -d ' ')"
violations=0

# ---- Apply rules -------------------------------------------------------------
while IFS= read -r line; do
  [ -z "$line" ] && continue
  case "$line" in \#*) continue ;; esac

  case "$line" in
    # ---- path:<glob>  — this file must never be committed at all ----
    path:*)
      glob="${line#path:}"
      while IFS= read -r f; do
        # shellcheck disable=SC2254
        case "$f" in $glob)
          echo "✗ forbidden path staged/committed: $f" >&2
          echo "    matches rule: $line" >&2
          violations=$((violations + 1)) ;;
        esac
      done < "$tmp_files"
      ;;

    # ---- re:<regex>  — forbidden content (regex) ----
    re:*)
      pat="${line#re:}"
      if [ "$MODE" = "staged" ]; then
        while IFS= read -r f; do
          hits="$(read_file "$f" | grep -nIE -e "$pat" 2>/dev/null || true)"
          if [ -n "$hits" ]; then
            echo "✗ boundary violation — pattern: $line  (in $f)" >&2
            printf '%s\n' "$hits" | sed 's/^/    /' >&2
            violations=$((violations + 1))
          fi
        done < "$tmp_files"
      else
        # shellcheck disable=SC2046,SC2086
        hits="$(grep -nIE -e "$pat" $(cat "$tmp_files") /dev/null 2>/dev/null || true)"
        if [ -n "$hits" ]; then
          echo "✗ boundary violation — denylisted pattern: $line" >&2
          printf '%s\n' "$hits" | sed 's/^/    /' >&2
          violations=$((violations + 1))
        fi
      fi
      ;;

    # ---- plain fixed string — forbidden content ----
    *)
      pat="$line"
      if [ "$MODE" = "staged" ]; then
        while IFS= read -r f; do
          hits="$(read_file "$f" | grep -nIF -e "$pat" 2>/dev/null || true)"
          if [ -n "$hits" ]; then
            echo "✗ boundary violation — pattern: $line  (in $f)" >&2
            printf '%s\n' "$hits" | sed 's/^/    /' >&2
            violations=$((violations + 1))
          fi
        done < "$tmp_files"
      else
        # shellcheck disable=SC2046,SC2086
        hits="$(grep -nIF -e "$pat" $(cat "$tmp_files") /dev/null 2>/dev/null || true)"
        if [ -n "$hits" ]; then
          echo "✗ boundary violation — denylisted pattern: $line" >&2
          printf '%s\n' "$hits" | sed 's/^/    /' >&2
          violations=$((violations + 1))
        fi
      fi
      ;;
  esac
done < "$DENYLIST"

# ---- Dependency manifests must not reference private portbay-cloud packages ---
for dep in package.json pnpm-lock.yaml package-lock.json yarn.lock \
           src-tauri/Cargo.toml src-tauri/Cargo.lock; do
  [ -f "$dep" ] || continue
  if hits="$(grep -nIE '@portbay-cloud/|portbay-cloud["'"'"']?[[:space:]]*[:=]' -- "$dep" 2>/dev/null)"; then
    echo "✗ private dependency reference in $dep:" >&2
    printf '%s\n' "$hits" | sed 's/^/    /' >&2
    violations=$((violations + 1))
  fi
done

if [ "$violations" -gt 0 ]; then
  {
    echo ""
    echo "$violations boundary check(s) failed (mode: $MODE)."
    echo "These belong ONLY in the private portbay-cloud repository, or must"
    echo "never be committed. See docs/architecture/repo-boundaries.md."
    echo "If a hit is documentation rather than a leak, add its path to"
    echo ".repo-boundary-allow (and be sure you are right)."
  } >&2
  exit 1
fi

echo "repo-boundary check: clean — mode: $MODE, $scanned files scanned"
