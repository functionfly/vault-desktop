#!/usr/bin/env bash
# Happy-path check for load-updater-env.sh, extracted into its own file so
# publish-update.test.sh does not have to nest three levels of quoting.
#
# Expects VAULT_KEY_DIR to point at a writable directory and
# LOAD_ENV_PATH to the loader under test. Both are supplied by the caller.
#
# This asserts the LOADER's contract, not the presence of any particular
# signing key. It previously sourced the loader with no VAULT_KEY_DIR
# override, which silently depended on load-updater-env.sh's default of
# <operator key dir>. That passed on the author's machine and failed on
# every CI runner with:
#
#   x load-updater-env.sh loads env (round-trip ok) (expected exit=0, got 1)
#
# because the loader correctly exits 1 when its key files are absent — that
# is check #6's contract, verified immediately before this one.
#
# A throwaway keypair is generated here. It is never uploaded, never used to
# sign a real artefact, and the directory is removed on exit.

set -euo pipefail

: "${VAULT_KEY_DIR:?VAULT_KEY_DIR must be set by the caller}"
: "${LOAD_ENV_PATH:?LOAD_ENV_PATH must be set by the caller}"

cleanup() { rm -rf "$VAULT_KEY_DIR"; }
trap cleanup EXIT

# The loader requires a password file it can read.
printf '%s' 'test-password' > "$VAULT_KEY_DIR/vault-key-password.txt"
chmod 600 "$VAULT_KEY_DIR/vault-key-password.txt"

# A real Ed25519 keypair so the loader's file-mode and readability checks
# behave exactly as they do in production. ssh-keygen writes both halves;
# the loader reads the *Tauri* pubkey file, so that one is replaced below.
ssh-keygen -q -t ed25519 -N '' -C 'ci-smoke-test' -f "$VAULT_KEY_DIR/vault-key.json"
chmod 600 "$VAULT_KEY_DIR/vault-key.json"
printf '%s' 'untrusted comment: minisign public key: CI test key' \
  > "$VAULT_KEY_DIR/vault-key.json.pub"
chmod 644 "$VAULT_KEY_DIR/vault-key.json.pub"

# shellcheck source=/dev/null
source "$LOAD_ENV_PATH" >/dev/null

test -n "${VAULT_UPDATER_PUBKEY:-}"
test -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}"
test -f "$TAURI_SIGNING_PRIVATE_KEY_PATH"
test "$(stat -c %a "$TAURI_SIGNING_PRIVATE_KEY_PATH")" = '600'

exit 0