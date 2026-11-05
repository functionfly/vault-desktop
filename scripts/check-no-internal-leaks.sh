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
#
# `docs/INTERNAL.md` is the documented home for the identifiers.
#
# THIS SCRIPT is also allowed, and has to be: it is a blocklist, so it
# necessarily contains the values it looks for. It is a list of
# identifiers, not credentials — the R2 secret access key is not in the
# repository — but it is still why `docs/INTERNAL.md` is the first thing
# to move when the repository is published.
#
# `.github/CODEOWNERS` must name real owners for branch protection to
# work at all, so it is allowed too.
ALLOWED=("docs/INTERNAL.md" "scripts/check-no-internal-leaks.sh" ".github/CODEOWNERS")

# Everything else in the repository is scanned.
#
# This used to be a hand-maintained list of "publishable" files. That
# list was incomplete, and an account id sat in
# `scripts/release/README.md` for a release without the guard noticing:
# the list is the guard's own weakest point, because a file nobody
# thought to add to it is silently exempt.
#
# So the whole tree is scanned now, minus build output, vendored
# dependencies and the explicitly allowed files above. There is no list
# to forget to update.
PUBLISHABLE=()
while IFS= read -r -d '' f; do
  PUBLISHABLE+=("$f")
done < <(
  find . \
    \( -path ./.git -o -path ./node_modules -o -path ./dist -o -path ./target \) -prune -o \
    \( -path ./src-tauri/target -o -path ./src-tauri/gen -o -path ./.release-please-manifest.json \) -prune -o \
    -type f \
    \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.mjs' \
       -o -name '*.sh' -o -name '*.py' -o -name '*.md' -o -name '*.toml' -o -name '*.json' \
       -o -name '*.yml' -o -name '*.yaml' -o -name '*.html' -o -name '*.css' \
       -o -name '*.txt' -o -name '*.plist' -o -name '*.conf' -o -name 'Dockerfile*' \) \
    -print0
)

# Strip the allowed files out of the scan set.
scan_set=()
for f in "${PUBLISHABLE[@]}"; do
  keep=1
  for a in "${ALLOWED[@]}"; do
    if [[ "$f" == "./$a" ]]; then keep=0; break; fi
  done
  (( keep )) && scan_set+=("$f")
done

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
    echo "ok  no $label outside the allowed files"
  fi
}

# 1. The Cloudflare R2 account id, in the exact form the scripts accept.
check "R2 account id" '<R2 account id — see the private runbook>' "${scan_set[@]}"

# 2. The dev-scoped R2 access key id.
check "R2 access key id" '<R2 access key id — see the private runbook>' "${scan_set[@]}"

# 3. The updater public-key fingerprint.
check "updater key fingerprint" '<updater key fingerprint — see the private runbook>' "${scan_set[@]}"

# 4. The public key material itself (base64 prefix).
#
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
# no use for it. The config file is the ONE place it belongs, so it is
# the ONE file exempt from this check — and scanned for every other
# marker instead.
pubkey_scan=()
for f in "${scan_set[@]}"; do
  [[ "$f" == "./src-tauri/tauri.conf.json" ]] && continue
  pubkey_scan+=("$f")
done
check "updater pubkey prefix" 'dW50cnVzdGVkIGNvbW1lbnQ6IG1vbmlzaWdu' "${pubkey_scan[@]}"

# 5. The maintainer's home directory, which leaks the build-host username.
check "operator home path" '<operator home>' "${scan_set[@]}"

# 6. Individual owner handles. CODEOWNERS and docs/INTERNAL.md are exempt:
#    CODEOWNERS must name real owners for branch protection to work at all,
#    and INTERNAL.md is the documented home for these values.
check "owner handle" '@functionflycom|@CallReef' "${scan_set[@]}"

# 7. The signing private key must never appear anywhere in the repo.
check "PRIVATE KEY block" 'BEGIN (OPENSSH |RSA |EC |)PRIVATE KEY' "${scan_set[@]}"

if (( fail != 0 )); then
  echo >&2
  echo "Internal-only material leaked into a publishable file." >&2
  exit 1
fi

echo
echo "No internal-only material outside the allowed files."
echo "(${ALLOWED[*]} are the documented homes for these values.)"
