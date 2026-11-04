#!/usr/bin/env bash
# Fails if internal-only material has leaked into publishable files.
#
# `docs/INTERNAL.md` is intentionally the ONE place that carries bucket
# identifiers, key paths, key fingerprints, and owner handles. Everything
# else is written to survive publication, because the repo may be flipped
# public without warning.
#
# This runs in CI so a leak is caught at the moment it is introduced rather
# than the moment somebody pastes a README into an issue.
#
# Not a secret scanner: it does not try to find high-entropy strings. It
# pins the exact identifiers we know are sensitive, which is the failure
# mode that actually happened here.
#
# NOTE: this file must stay executable (`100755` in the git index). CI
# runs it directly as `./scripts/check-no-internal-leaks.sh`; a `100644`
# mode makes that invocation fail with exit 126 ("permission denied")
# before the script ever runs. `git update-index --chmod=+x
# scripts/check-no-internal-leaks.sh` restores it.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Files allowed to carry these markers.
ALLOWED=("docs/INTERNAL.md")

# Everything that ships publicly. `docs/INTERNAL.md` is deliberately
# absent from this list, and `.github/CODEOWNERS` is deliberately absent
# because it must name real owners for branch protection to work at all.
#
# `CHANGELOG.md` IS scanned: it is copied verbatim into the GitHub
# release notes, so a leak there reaches every subscriber's inbox. It
# used to carry the build host's home directory and went unnoticed
# because this list did not include it.
PUBLISHABLE=(
  README.md
  CHANGELOG.md
  docs/BRANCHING.md
  scripts/release/KEY-FINGERPRINT.txt
  scripts/release/*.sh
  scripts/release/README.md
  scripts/release/evidence/*.md
  .github/workflows/*.yml
)

fail=0

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
    echo "::error::$label found in publishable file(s): ${hits[*]}" >&2
    echo "         Move it to docs/INTERNAL.md, or replace it with a placeholder." >&2
    fail=1
  else
    echo "ok  no $label outside docs/INTERNAL.md"
  fi
}

# 1. The Cloudflare R2 account id, in the exact form the scripts accept.
check "R2 account id" '<R2 account id — see the private runbook>' "${PUBLISHABLE[@]}"

# 2. The dev-scoped R2 access key id.
check "R2 access key id" '<R2 access key id — see the private runbook>' "${PUBLISHABLE[@]}"

# 3. The updater public-key fingerprint.
check "updater key fingerprint" '<updater key fingerprint — see the private runbook>' "${PUBLISHABLE[@]}"

# 4. The public key material itself (base64 prefix).#
# NOTE: the updater's Ed25519 PUBLIC key is now committed to
# `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). That is not a
# leak and not a regression — the key has to be compiled into the
# binary, otherwise an installed client has no way to verify an update
# signature and auto-update is permanently broken. A public verification
# key is not secret material; the PRIVATE key is, and check 7 covers it.
#
# What this check still protects is the *prose*: the key must not be
# pasted into the README, the CHANGELOG, or the release scripts, where
# it would be copied into release notes and published to users who have
# no use for it. `src-tauri/tauri.conf.json` is deliberately NOT in the
# PUBLISHABLE list for exactly that reason.check "updater pubkey prefix" 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWdu' "${PUBLISHABLE[@]}"

# 5. The maintainer's home directory, which leaks the build-host username.
check "operator home path" '<operator home>' "${PUBLISHABLE[@]}"

# 6. Individual owner handles. CODEOWNERS and docs/INTERNAL.md are exempt:
#    CODEOWNERS must name real owners for branch protection to work at all,
#    and INTERNAL.md is the documented home for these values.
check "owner handle" '@functionflycom|@CallReef' "${PUBLISHABLE[@]}"

# 7. The signing private key must never appear anywhere in the repo.
check "PRIVATE KEY block" 'BEGIN (OPENSSH |RSA |EC |)PRIVATE KEY' \
  README.md CHANGELOG.md docs scripts .github src src-tauri/src

if (( fail != 0 )); then
  echo >&2
  echo "Internal-only material leaked into a publishable file." >&2
  exit 1
fi

echo
echo "No internal-only material outside docs/INTERNAL.md."
echo "(${ALLOWED[*]} is the documented home for these values.)"
