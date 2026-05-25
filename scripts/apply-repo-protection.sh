#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# apply-repo-protection.sh — apply PortBay's GitHub hardening to the public repo.
#
# Idempotent: re-running updates the existing rulesets/settings rather than
# duplicating them. Defaults to DRY-RUN (prints what it would do); pass --apply
# to actually mutate the live repository. Requires the GitHub CLI (`gh`),
# authenticated as a user with admin on the repo.
#
# Usage:
#   scripts/apply-repo-protection.sh                 # dry run
#   scripts/apply-repo-protection.sh --apply         # apply for real
#   scripts/apply-repo-protection.sh --repo portbay-app/portbay --apply
#
# What it sets:
#   * Branch ruleset on the default branch: require PR (1 approval, CODEOWNERS,
#     stale-dismissal, last-push approval, thread resolution), all CI/governance
#     status checks must pass (strict/up-to-date), linear history, no force-push,
#     no deletion, squash/rebase only, no bypass actors.
#   * Tag ruleset on v*: immutable release tags (no delete/update/force).
#   * Actions: default workflow token = read-only; Actions cannot approve PRs.
#   * Secret scanning + push protection, Dependabot alerts + security fixes,
#     private vulnerability reporting.

set -euo pipefail

APPLY=0
REPO=""
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --dry-run) APPLY=0 ;;
    --repo) REPO="$2"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

command -v gh >/dev/null 2>&1 || { echo "error: GitHub CLI (gh) not installed — https://cli.github.com" >&2; exit 2; }
gh auth status >/dev/null 2>&1 || { echo "error: not logged in — run: gh auth login" >&2; exit 2; }

# Resolve owner/repo from the git remote if not provided.
if [ -z "$REPO" ]; then
  url="$(git config --get remote.origin.url || true)"
  REPO="$(printf '%s' "$url" | sed -E 's#(git@github.com:|https://github.com/)##; s#\.git$##')"
fi
[ -n "$REPO" ] || { echo "error: could not determine owner/repo; pass --repo owner/name" >&2; exit 2; }

ROOT="$(git rev-parse --show-toplevel)"
RULES_DIR="$ROOT/.github/rulesets"

echo "Repo:     $REPO"
echo "Mode:     $([ "$APPLY" -eq 1 ] && echo APPLY || echo DRY-RUN)"
echo "Admin:    $(gh api "repos/$REPO" --jq '.permissions.admin' 2>/dev/null || echo 'unknown')"
echo ""

run() {
  if [ "$APPLY" -eq 1 ]; then
    "$@"
  else
    printf '  [dry-run] '; printf '%q ' "$@"; printf '\n'
  fi
}

upsert_ruleset() {
  local file="$1" name
  name="$(grep -m1 '"name"' "$file" | sed -E 's/.*"name"[^"]*"([^"]+)".*/\1/')"
  echo "• ruleset: $name  ($file)"
  local existing
  existing="$(gh api "repos/$REPO/rulesets" --jq ".[] | select(.name==\"$name\") | .id" 2>/dev/null | head -1 || true)"
  if [ -n "$existing" ]; then
    run gh api -X PUT "repos/$REPO/rulesets/$existing" --input "$file"
  else
    run gh api -X POST "repos/$REPO/rulesets" --input "$file"
  fi
}

echo "== Branch & tag rulesets =="
upsert_ruleset "$RULES_DIR/main-protection.json"
upsert_ruleset "$RULES_DIR/release-tags.json"
echo ""

echo "== Actions hardening =="
run gh api -X PUT "repos/$REPO/actions/permissions/workflow" \
  -f default_workflow_permissions=read \
  -F can_approve_pull_request_reviews=false
echo ""

echo "== Security features =="
run gh api -X PUT "repos/$REPO/vulnerability-alerts"
run gh api -X PUT "repos/$REPO/automated-security-fixes"
run gh api -X PUT "repos/$REPO/private-vulnerability-reporting"

# Nested object → must be sent as a JSON body (gh -f bracket syntax does NOT
# build nested JSON). Write it to a temp file and pass with --input.
SEC_JSON="$(mktemp)"
trap 'rm -f "$SEC_JSON"' EXIT
cat > "$SEC_JSON" <<'JSON'
{"security_and_analysis":{"secret_scanning":{"status":"enabled"},"secret_scanning_push_protection":{"status":"enabled"}}}
JSON
run gh api -X PATCH "repos/$REPO" --input "$SEC_JSON"
echo ""

if [ "$APPLY" -eq 1 ]; then
  echo "Applied. Verify in Settings → Rules → Rulesets and Settings → Code security."
else
  echo "Dry run complete. Re-run with --apply to make changes."
  echo "Manual steps gh cannot reliably set are listed in docs/maintainers/repo-hardening.md."
fi
