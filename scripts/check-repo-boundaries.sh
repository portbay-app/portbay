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
#   (default)      Scan tracked + untracked-not-ignored files. Used by CI.
#                  Catches a leak anywhere in the working tree.
#   --staged       Scan only what is staged for commit (the exact blobs that
#                  would enter history). Used by the `pre-commit` hook so a bad
#                  commit is blocked BEFORE it is created — no force-clean, ever.
#   --ref <commit> Scan the tree of <commit> (the exact blobs a push would
#                  publish). Used by `pre-push` per outgoing ref, so local
#                  working-tree WIP (e.g. private-overlay files sitting
#                  uncommitted in the tree) can never false-block — and a
#                  forbidden file that IS in the pushed tree always blocks,
#                  whether or not it exists locally.
#
# Exit: 0 = clean   1 = boundary violation(s)   2 = misconfiguration
#
# Written for bash 3.2 (macOS / GitHub-runner default). Assumes tracked/staged
# paths contain no spaces; it errors clearly if that ever stops holding.

set -euo pipefail

MODE="full"
REF=""
case "${1:-}" in
  --staged) MODE="staged" ;;
  --ref)
    MODE="ref"
    REF="${2:-}"
    if [ -z "$REF" ] || ! git rev-parse --verify --quiet "${REF}^{commit}" >/dev/null; then
      echo "usage: $0 --ref <commit>  (got: '${REF}')" >&2
      exit 2
    fi
    ;;
  "" ) ;;
  * ) echo "usage: $0 [--staged | --ref <commit>]" >&2; exit 2 ;;
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
elif [ "$MODE" = "ref" ]; then
  # The committed tree being pushed — never the working tree.
  raw_list() { git ls-tree -r --name-only "$REF"; }
  read_file() { git show "$REF:$1" 2>/dev/null || true; }
else
  raw_list() { git ls-files --cached --others --exclude-standard; }
  read_file() { cat "$1" 2>/dev/null || true; }
fi

# Content scan over the (allow-filtered) candidate list in one subprocess per
# rule: plain grep on working files, `git grep <ref>` on a committed tree.
# $1 = E|F (regex / fixed string), $2 = pattern. Empty list ⇒ no hits.
grep_candidates() {
  [ -s "$tmp_files" ] || return 0
  if [ "$MODE" = "ref" ]; then
    # shellcheck disable=SC2046
    git grep -nI"$1" -e "$2" "$REF" -- $(cat "$tmp_files") 2>/dev/null || true
  else
    # shellcheck disable=SC2046
    grep -nI"$1" -e "$2" $(cat "$tmp_files") /dev/null 2>/dev/null || true
  fi
}

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
        hits="$(grep_candidates E "$pat")"
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
        hits="$(grep_candidates F "$pat")"
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
dep_pat='@portbay-cloud/|portbay-cloud["'"'"']?[[:space:]]*[:=]'
for dep in package.json pnpm-lock.yaml package-lock.json yarn.lock \
           src-tauri/Cargo.toml src-tauri/Cargo.lock; do
  if [ "$MODE" = "ref" ]; then
    git cat-file -e "$REF:$dep" 2>/dev/null || continue
    hits="$(read_file "$dep" | grep -nIE "$dep_pat" 2>/dev/null || true)"
  else
    [ -f "$dep" ] || continue
    hits="$(grep -nIE "$dep_pat" -- "$dep" 2>/dev/null || true)"
  fi
  if [ -n "$hits" ]; then
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
