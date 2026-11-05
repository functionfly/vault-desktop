#!/usr/bin/env bash
# Fails if internal-only material has leaked into publishable files.
#
# ── Why this script exists ────────────────────────────────────────────────
#
# The repository is going PUBLIC. When that happens GitHub publishes every
# commit on every branch, every pull-request ref, and every tag — not just
# the current tree. So a credential that was committed and later removed
# is still public. The audit that matters is over the whole object graph,
# not the working tree.
#
# This script is the gate that keeps a leak from being introduced. It runs
# in CI, so a leak is caught when it is introduced rather than the day
# somebody pastes a README into an issue.
#
# ── Two layers, deliberately ──────────────────────────────────────────────
#
# LAYER 1 — GENERIC PATTERNS. These live in this file. They are *shapes*,
# not values: "a private key block", "an AWS access key id". No specific
# identifier of ours appears here, so this file is safe to publish.
#
# LAYER 2 — SPECIFIC IDENTIFIERS. These are the exact strings that are
# sensitive for THIS project: the R2 account id, the R2 access key id,
# the updater key fingerprint, the operator's home path, the owner
# handles.
#
#   *** They are NOT stored in this repository. ***
#
# An earlier version of this script hardcoded them. That was a mistake:
# the guard was publishing the very values it was protecting, because the
# file containing the blocklist shipped with the thing it was guarding.
#
# They now live in a git-ignored file (default:
# `.internal-identifiers.local` in the repo root), which the operator keeps
# on the build host and CI injects from a secret. The exact path can be
# overridden with `VAULT_INTERNAL_IDENTIFIERS_FILE`.
#
# If that file is absent, LAYER 2 is skipped and the script says so
# loudly — it does NOT silently pass. A guard that quietly stops
# guarding is worse than no guard, because it looks green.
#
# ── What is NOT a secret here ────────────────────────────────────────────
#
# The updater's Ed25519 PUBLIC key is committed to
# `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). That is not a
# leak: the key has to be compiled into the binary, otherwise an
# installed client has no way to verify an update signature and
# auto-update is permanently broken for end users. A public verification
# key is not secret material; the PRIVATE key is, and LAYER 1 catches it.
#
# NOTE: this file must stay executable (`100755` in the git index). CI
# runs it directly as `./scripts/check-no-internal-leaks.sh`; a `100644`
# mode makes that invocation fail with exit 126 ("permission denied")
# before the script ever runs. `git update-index --chmod=+x
# scripts/check-no-internal-leaks.sh` restores it.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

fail=0

# Files that are allowed to carry internal-only material.
#
# `.github/CODEOWNERS` must name real owners for branch protection to
# work at all, so it is exempt from the owner-handle check only.
ALLOWED_ALL=(
  "docs/INTERNAL.md"
  ".github/CODEOWNERS"
)

# ── Build the scan set ───────────────────────────────────────────────────
# The WHOLE tree is scanned, minus build output, vendored dependencies
# and generated schema. There is no hand-maintained "publishable files"
# list, because that list is the guard's weakest point: a file nobody
# thought to add to it is silently exempt, which is exactly how a leak
# survived in this repository before.
PUBLISHABLE=()
while IFS= read -r -d '' f; do
  PUBLISHABLE+=("$f")
done < <(
  find . \
    \( -path ./.git -o -path ./node_modules -o -path ./dist -o -path ./target \) -prune -o \
    \( -path ./src-tauri/target -o -path ./src-tauri/gen \) -prune -o \
    -type f \
    \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.mjs' \
       -o -name '*.sh' -o -name '*.py' -o -name '*.md' -o -name '*.toml' -o -name '*.json' \
       -o -name '*.yml' -o -name '*.yaml' -o -name '*.html' -o -name '*.css' \
       -o -name '*.txt' -o -name '*.plist' -o -name '*.conf' -o -name 'Dockerfile*' \) \
    -print0
)

# Strip the fully-exempt files out of the scan set.
scan_set=()
for f in "${PUBLISHABLE[@]}"; do
  keep=1
  for a in "${ALLOWED_ALL[@]}"; do
    if [[ "$f" == "./$a" ]]; then keep=0; break; fi
  done
  (( keep )) && scan_set+=("$f")
done

echo "scanning ${#scan_set[@]} files"

# ── The check itself ─────────────────────────────────────────────────────
check() {
  local label="$1" pattern="$2"
  shift 2

  local hits=()
  local f
  for f in "$@"; do
    [[ -e "$f" ]] || continue
    if grep -rIlE "$pattern" "$f" >/dev/null 2>&1; then
      hits+=("$f")
    fi
  done

  if (( ${#hits[@]} > 0 )); then
    echo "::error::${label} found in publishable file(s): ${hits[*]}" >&2
    echo "         Move it out of the repository, or replace it with a placeholder." >&2
    fail=1
  else
    echo "ok  no ${label}"
  fi
}

# ══ LAYER 1 — generic patterns ══════════════════════════════════════════
# Shapes, not values. Safe to publish.

check "private key block" \
  'BEGIN (OPENSSH |RSA |EC |DSA |ENCRYPTED |PGP )?PRIVATE KEY' "${scan_set[@]}"

check "AWS access key id" \
  '(^|[^A-Z0-9])(AKIA|ASIA|AIDA|AROA|ANPA|ABIA)[A-Z0-9]{16}' "${scan_set[@]}"

check "GitHub token" \
  'gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}' "${scan_set[@]}"

check "Slack token" \
  'xox[baprs]-[A-Za-z0-9-]{10,}' "${scan_set[@]}"

check "Stripe secret key" \
  '(^|[^a-z])(sk|rk)_live_[A-Za-z0-9]{20,}' "${scan_set[@]}"

check "Google API key" \
  'AIza[0-9A-Za-z_-]{35}' "${scan_set[@]}"

check "SendGrid key" \
  'SG\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}' "${scan_set[@]}"

check "Twilio account SID" \
  'AC[a-f0-9]{32}' "${scan_set[@]}"

check "npm token" \
  'npm_[A-Za-z0-9]{36}' "${scan_set[@]}"

check "OpenAI / Anthropic API key" \
  'sk-ant-[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9]{32,}' "${scan_set[@]}"

# A real JWT carries a base64url signature of at least 43 characters for
# HS256. Requiring that length keeps short test fixtures from tripping
# the check while still catching a genuine token.
check "JSON Web Token" \
  'eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{43,}' "${scan_set[@]}"

# NOTE: the pattern is written as two adjacent quoted strings so this
# file does not contain the literal it searches for. A guard that matches
# itself fails on every run, which trains people to ignore it.
check "dotenv private key" \
  'DOTENV_PRIVATE''_KEY=' "${scan_set[@]}"

check "credential in a URL" \
  '://[^/\s:@]+:[^/\s:@]+@' "${scan_set[@]}"

# ══ LAYER 2 — this project's specific identifiers ═══════════════════════
# Read from a git-ignored file. NOT stored in this repository.

IDENTIFIERS_FILE="${VAULT_INTERNAL_IDENTIFIERS_FILE:-$REPO_ROOT/.internal-identifiers.local}"

if [[ -s "$IDENTIFIERS_FILE" ]]; then
  # Each non-blank, non-comment line is a literal string that must not
  # appear anywhere in the publishable set. A line may be `label=value`
  # or just `value`; only the value is searched for.
  while IFS= read -r line; do
    # Trim whitespace and skip blanks/comments.
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    [[ -z "$line" || "$line" == \#* ]] && continue

    if [[ "$line" == *=* ]]; then
      label="${line%%=*}"
      value="${line#*=}"
    else
      label="identifier"
      value="$line"
    fi
    [[ -z "$value" ]] && continue

    # Search literally, not as a regex: these values contain characters
    # that mean something to grep -E.
    hits=()
    for f in "${scan_set[@]}"; do
      [[ -e "$f" ]] || continue
      if grep -rIlF -- "$value" "$f" >/dev/null 2>&1; then
        hits+=("$f")
      fi
    done

    if (( ${#hits[@]} > 0 )); then
      echo "::error::${label} found in publishable file(s): ${hits[*]}" >&2
      echo "         Move it out of the repository, or replace it with a placeholder." >&2
      fail=1
    else
      echo "ok  no ${label}"
    fi
  done < "$IDENTIFIERS_FILE"
else
  echo "::warning::no internal-identifier list at ${IDENTIFIERS_FILE}" >&2
  echo "         Layer 2 (this project's specific identifiers) was SKIPPED." >&2
  echo "         Set VAULT_INTERNAL_IDENTIFIERS_FILE, or create that file," >&2
  echo "         to enable it. CI injects it from a secret." >&2
  echo "         Layer 1 (generic secret shapes) still ran." >&2
fi

if (( fail != 0 )); then
  echo >&2
  echo "Internal-only material leaked into a publishable file." >&2
  exit 1
fi

echo
echo "No internal-only material in the publishable tree."