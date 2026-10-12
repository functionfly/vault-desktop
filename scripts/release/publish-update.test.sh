#!/usr/bin/env bash
# Smoke test for scripts/release/. Validates arg parsing, channel
# validation, version validation, and pre-flight env checks WITHOUT
# touching R2. Operators should run this after editing
# `publish-update.sh` to catch regressions before a real publish.
#
# Strategy: re-run the script's `usage()` + arg validation paths by
# feeding each invalid input in turn and asserting the script exits
# non-zero with the expected error category.

set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$DIR/publish-update.sh"
LOAD_ENV="$DIR/load-updater-env.sh"

fail=0
expect_exit() {
  local description="$1"
  local expected="$2"
  shift 2
  local actual=0
  "$@" >/dev/null 2>&1 || actual=$?
  if [[ "$actual" -eq "$expected" ]]; then
    echo "  ✓ $description"
  else
    echo "  ✗ $description (expected exit=$expected, got $actual)"
    fail=1
  fi
}

echo "── scripts/release smoke test ──"
echo

# 1. Bash + shellcheck already passed at write-time; re-confirm here.
echo "→ syntax"
expect_exit "publish-update.sh is bash-clean" 0 bash -n "$SCRIPT"
expect_exit "load-updater-env.sh is bash-clean" 0 bash -n "$LOAD_ENV"

# 2. Usage on missing args
echo
echo "→ usage"
expect_exit "zero args → exit 64" 64 "$SCRIPT"

# 3. Bad channel
expect_exit "invalid channel → exit 1" 1 \
  "$SCRIPT" "release" "0.1.1" "/dev/null" "/dev/null"

# 4. Bad version
expect_exit "invalid version → exit 1" 1 \
  "$SCRIPT" "stable" "not-a-version" "/dev/null" "/dev/null"

# 5. Missing env (no R2_*) → exit 1 (pre-flight fail)
echo
echo "→ pre-flight"
expect_exit "missing R2_ACCOUNT_ID → exit 1" 1 \
  env -i PATH="$PATH" "$SCRIPT" "stable" "0.1.1" "/dev/null" "/dev/null"

# 6. load-updater-env.sh: missing private key
expect_exit "load-updater-env.sh without key files → exit 1" 1 \
  env -i PATH="$PATH" VAULT_KEY_DIR="/nonexistent" bash -c "source $LOAD_ENV"

# 7. load-updater-env.sh: happy path (uses the real key we generated)
#
# This check asserts the loader's contract — that sourcing it exports a
# non-empty pubkey, a private-key path, and that the path exists and is
# mode 0600 — WITHOUT requiring the real signing key to be present.
#
# It previously sourced the loader with no VAULT_KEY_DIR override, which
# silently depended on `load-updater-env.sh`'s default of
# <operator key dir>. That passed on the author's machine and failed on
# every CI runner with:
#
#   x load-updater-env.sh loads env (round-trip ok) (expected exit=0, got 1)
#
# because the loader correctly exits 1 when the key files are absent (that
# is check #6's contract, verified just above).
#
# A throwaway Ed25519 keypair is generated into a temp dir instead. It is
# never uploaded, never used to sign a real artefact, and is deleted on
# exit. This keeps the check hermetic: it tests the loader, not whether a
# particular machine happens to hold a key.
expect_exit "load-updater-env.sh loads env (round-trip ok)" 0 \
  env VAULT_KEY_DIR="$(mktemp -d)" LOAD_ENV_PATH="$LOAD_ENV" \
  bash "${DIR}/_load-env-smoke.sh"

# 8. (Optional) End-to-end round-trip against the bundled mock R2.
#    Skipped unless `MOCK_R2_STORE` is set, `python3` is on PATH,
#    and `aws` is installed. Captured evidence at
#    `scripts/release/evidence/2026-10-06-mock-r2-round-trip.md`.
if [[ -n "${MOCK_R2_STORE:-}" ]] && command -v python3 >/dev/null 2>&1 && command -v aws >/dev/null 2>&1; then
  echo
  echo "→ mock R2 round-trip (optional)"
  MOCK_DIR="$DIR/_mock-r2"
  MOCK_PORT=18235
  rm -rf "$MOCK_DIR"
  MOCK_R2_STORE="$MOCK_DIR" python3 "$DIR/mock-r2.py" "$MOCK_PORT" >/dev/null 2>&1 &
  MOCK_PID=$!
  # Note: $MOCK_PID must be expanded at trap-registration time (single quotes).
  trap 'kill "$MOCK_PID" 2>/dev/null || true' EXIT
  # Wait for the mock to start listening
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -o /dev/null "http://127.0.0.1:${MOCK_PORT}/healthcheck" 2>/dev/null; then
      break
    fi
    sleep 0.2
  done

  FIXTURE_DIR="$DIR/_fixtures"
  mkdir -p "$FIXTURE_DIR"
  FIXTURE_SIDECAR="$FIXTURE_DIR/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz"
  FIXTURE_SIG="$FIXTURE_DIR/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig"
  if [[ ! -s "$FIXTURE_SIDECAR" ]]; then head -c 4096 /dev/urandom > "$FIXTURE_SIDECAR"; fi
  if [[ ! -s "$FIXTURE_SIG" ]]; then head -c 128 /dev/urandom > "$FIXTURE_SIG"; fi

  if bash -c "source $LOAD_ENV && \
      R2_ACCOUNT_ID=mock R2_ACCESS_KEY_ID=FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE \
      R2_SECRET_ACCESS_KEY=fakefakesecret_secret_secret_secret_secret_secret_see \
      R2_ENDPOINT_OVERRIDE=http://127.0.0.1:${MOCK_PORT} \
      $SCRIPT stable 0.0.1 \
        $FIXTURE_SIDECAR $FIXTURE_SIG" >/dev/null 2>&1; then
    if [[ -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz" \
       && -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig" \
       && -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/manifest.json" ]]; then
      echo "  ✓ publish-update.sh uploaded side-car + .sig + manifest to mock R2"
    else
      echo "  ✗ publish-update.sh succeeded but expected objects missing on the mock"
      fail=1
    fi
  else
    echo "  ✗ publish-update.sh failed against the mock R2"
    fail=1
  fi
  rm -rf "$FIXTURE_DIR"
fi

echo
if [[ $fail -eq 0 ]]; then
  echo "All smoke checks green ✓"
  exit 0
else
  echo "Smoke checks failed ✗"
  exit 1
fi