#!/usr/bin/env bash
# keychain-smoke.sh — automates the per-OS keychain check from the
# launch checklist in README.md.
#
# WHAT IT PROVES
# --------------
# The vault stores the long-lived session in the operating system's
# credential store (Windows Credential Manager / macOS Keychain /
# Linux Secret Service). If that path is broken, the app silently falls
# back to the encrypted file store — the user gets a weaker store and is
# never told, because the fallback is a deliberate, documented behaviour
# for headless boxes.
#
# This script therefore checks two separate things:
#
#   1. The application's own `SecretStore` code round-trips a session
#      through the REAL OS keychain (not the file fallback).
#      -> `cargo test --test keychain_smoke`
#
#   2. The credential is visible through the platform's own tooling, so
#      an operator can reproduce the check by hand.
#      -> cmdkey / security / secret-tool
#
# PLATFORM NOTES
# --------------
# The credential NAME differs per platform, which is why the manual
# checklist was easy to get wrong:
#
#   Windows  The `keyring` crate stores a generic credential whose target
#            is "{account}.{service}", so the target is
#            "orchestrator-access-token.functionfly-vault-desktop".
#            `cmdkey /list:functionfly-vault-desktop` matches NOTHING and
#            reports "* NONE *" — it looks like a broken keychain when it
#            is working perfectly.
#
#   macOS    The service becomes the Keychain service attribute, so
#            `security find-generic-password -s functionfly-vault-desktop`
#            is correct.
#
#   Linux    The service becomes the Secret Service `service` attribute,
#            so `secret-tool search service functionfly-vault-desktop` is
#            correct. It needs a running Secret Service, which a headless
#            box does not have — the Rust test skips there, loudly.
#
# USAGE
# -----
#   ./scripts/keychain-smoke.sh
#
# Exit codes:
#   0  the OS keychain round-tripped the session
#   1  the round-trip failed, or the credential was not visible to the
#      platform tooling
#   2  configuration error

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CRATE_DIR="src-tauri"
STORE_RS="$CRATE_DIR/src/store.rs"

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*" >&2; exit "${2:-1}"; }
note() { printf "  · %s\n" "$*"; }

[ -f "$STORE_RS" ] || fail "$STORE_RS not found" 2

# ── Read the credential names straight from the Rust source ───────────────
# Deriving them here means this script cannot drift away from the
# application: renaming a constant breaks the script loudly instead of
# silently checking for a credential that no longer exists.
read_const() {
  local name="$1"
  local value
  value="$(sed -n "s/^pub const ${name}: &str = \"\(.*\)\";$/\1/p" "$STORE_RS" | head -1)"
  if [ -z "$value" ]; then
    fail "could not read '$name' from $STORE_RS — the script needs updating" 2
  fi
  printf '%s' "$value"
}

KEYRING_SERVICE="$(read_const KEYRING_SERVICE)"
KEY_USER_ACCESS="$(read_const KEY_USER_ACCESS)"
KEY_USER_REFRESH="$(read_const KEY_USER_REFRESH)"

note "service       = ${KEYRING_SERVICE}"
note "accounts      = ${KEY_USER_ACCESS}, ${KEY_USER_REFRESH}"

# ── Platform detection ────────────────────────────────────────────────────
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) PLATFORM="windows" ;;
  Darwin)                PLATFORM="macos" ;;
  Linux)                 PLATFORM="linux" ;;
  *) PLATFORM="unknown" ;;
esac

note "platform      = ${PLATFORM}"

# ── 1. Round-trip through the application's own store code ────────────────
step "1. OS keychain round-trip (cargo test --test keychain_smoke)"

# The test asserts that `BackendKind::OsKeychain` was the backend actually
# used. On Windows and macOS a file fallback is a hard failure; on Linux it
# is a skip, because a headless box legitimately has no Secret Service.
#
# VAULT_KEYCHAIN_PROBE_KEEP=1 leaves the credential in place so step 2 can
# confirm it through the platform's own tooling.
export VAULT_KEYCHAIN_PROBE_KEEP=1

if ! cargo test --manifest-path "$CRATE_DIR/Cargo.toml" --test keychain_smoke -- --nocapture; then
  fail "the keychain round-trip test failed" 1
fi

# ── 2. Confirm the credential is visible to the platform tooling ──────────
step "2. Confirm the credential through the platform tooling"

case "$PLATFORM" in
  windows)
    # The keyring crate's Windows target is "{account}.{service}".
    TARGET="${KEY_USER_ACCESS}.${KEYRING_SERVICE}"
    note "cmdkey /list:${TARGET}"
    LIST_OUTPUT="$(cmdkey.exe "/list:${TARGET}" 2>&1 || true)"
    printf '%s\n' "$LIST_OUTPUT" | sed 's/^/      /'
    if printf '%s' "$LIST_OUTPUT" | grep -q "^\* NONE \*"; then
      fail "cmdkey found no credential for '${TARGET}'" 1
    fi
    if ! printf '%s' "$LIST_OUTPUT" | grep -qi "functionfly-vault-desktop"; then
      fail "cmdkey output did not mention the vault service" 1
    fi
    ok "the credential is present in Windows Credential Manager"
    ;;

  macos)
    note "security find-generic-password -s ${KEYRING_SERVICE}"
    if ! security find-generic-password -s "$KEYRING_SERVICE" >/dev/null 2>&1; then
      fail "the Keychain has no entry for service '${KEYRING_SERVICE}'" 1
    fi
    ok "the credential is present in the macOS Keychain"
    ;;

  linux)
    if ! command -v secret-tool >/dev/null 2>&1; then
      note "secret-tool is not installed; skipping the Secret Service check."
      note "Install 'libsecret-tools' to verify the Linux keychain."
    else
      note "secret-tool search service ${KEYRING_SERVICE}"
      if ! secret-tool search service "$KEYRING_SERVICE" >/dev/null 2>&1; then
        fail "the Secret Service has no entry for service '${KEYRING_SERVICE}'" 1
      fi
      ok "the credential is present in the Secret Service"
    fi
    ;;

  *)
    note "unknown platform; skipping the credential listing check."
    ;;
esac

# ── 3. Clean up ───────────────────────────────────────────────────────────
step "3. Clean up the probe credential"

unset VAULT_KEYCHAIN_PROBE_KEEP
if ! cargo test --manifest-path "$CRATE_DIR/Cargo.toml" --test keychain_smoke; then
  fail "the cleanup pass failed" 1
fi

case "$PLATFORM" in
  windows)
    cmdkey.exe "/delete:${KEY_USER_ACCESS}.${KEYRING_SERVICE}" >/dev/null 2>&1 || true
    cmdkey.exe "/delete:${KEY_USER_REFRESH}.${KEYRING_SERVICE}" >/dev/null 2>&1 || true
    ;;
  macos)
    security delete-generic-password -s "$KEYRING_SERVICE" >/dev/null 2>&1 || true
    ;;
  linux)
    if command -v secret-tool >/dev/null 2>&1; then
      secret-tool clear service "$KEYRING_SERVICE" >/dev/null 2>&1 || true
    fi
    ;;
esac

step "summary"
printf "  platform   %s\n" "$PLATFORM"
printf "  service    %s\n" "$KEYRING_SERVICE"
printf "  result     \033[1;32mOS keychain verified\033[0m\n"
