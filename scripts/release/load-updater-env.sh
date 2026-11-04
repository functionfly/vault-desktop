#!/usr/bin/env bash
# Source this file in the build host's CI environment to expose
# `VAULT_UPDATER_PUBKEY` (and the matching Tauri signing env vars) to
# the build pipeline.
#
# The four operator follow-up steps (README §"Updater" + CHANGELOG
# 2026-10-06) are:
#   1. `cargo-tauri signer generate`     ← run once
#   2. this file                         ← source in CI
#   3. `provision-r2-bucket.sh`          ← run once per env
#   4. `publish-update.sh`               ← run on every release
#
# Layout assumptions (override with the env vars at the top):
#   - Private key + password:    $VAULT_KEY_DIR/vault-key{,.json}
#   - Public key (this file):    $VAULT_KEY_DIR/vault-key.json.pub
#   - Build cwd:                 $VAULT_DESKTOP_DIR  (this repo's root)
#
# Side effects:
#   - Exports VAULT_UPDATER_PUBKEY, TAURI_SIGNING_PRIVATE_KEY,
#     TAURI_SIGNING_PRIVATE_KEY_PATH, TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#   - Exposes the canonical R2 endpoint URLs (overridable per channel)
#   - Records the key fingerprint to stdout so the operator can pin
#     it in the release notes

set -euo pipefail

# ── Configurable inputs ───────────────────────────────────────────────
: "${VAULT_KEY_DIR:=$HOME/.tauri}"
# Default to this script's own repo root (scripts/release -> ../..) so the
# path stays correct wherever the repo is cloned. Sourced from either bash or
# zsh: bash sets BASH_SOURCE[0] but leaves $0 as the shell name, while zsh
# leaves BASH_SOURCE unset and puts the sourced path in $0.
: "${VAULT_DESKTOP_DIR:=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}"
: "${VAULT_KEY_PASSWORD_FILE:=$VAULT_KEY_DIR/vault-key-password.txt}"
: "${VAULT_PUBKEY_FILE:=$VAULT_KEY_DIR/vault-key.json.pub}"
# The endpoint is the channel's static manifest. `tauri-plugin-updater`
# only substitutes `{{current_version}}`, `{{target}}`, `{{arch}}` and
# `{{bundle_type}}` — there is no `{{target_version}}` placeholder, and
# the previous default left that token in the request URL verbatim,
# which no object in the bucket could ever satisfy.
: "${VAULT_UPDATER_STABLE_URL:=https://updates.functionfly.com/vault/stable/manifest.json}"
: "${VAULT_UPDATER_BETA_URL:=https://updates.functionfly.com/vault/beta/manifest.json}"

# ── Pre-flight checks (fail closed with a clear message) ──────────────
for f in "$VAULT_KEY_DIR/vault-key.json" "$VAULT_PUBKEY_FILE" "$VAULT_KEY_PASSWORD_FILE"; do
  if [[ ! -s "$f" ]]; then
    echo "load-updater-env: missing or empty: $f" >&2
    echo "  run 'cargo-tauri signer generate -w $VAULT_KEY_DIR/vault-key.json' first" >&2
    exit 1
  fi
done

# Private key file should never be world-readable.
priv_mode=$(stat -c '%a' "$VAULT_KEY_DIR/vault-key.json")
if [[ "$priv_mode" != "600" && "$priv_mode" != "400" ]]; then
  echo "load-updater-env: private key mode is $priv_mode, want 600 or 400" >&2
  exit 1
fi

# ── Export env vars for cargo-tauri (signing) + tauri-plugin-updater ──
export VAULT_UPDATER_PUBKEY
VAULT_UPDATER_PUBKEY=$(tr -d '\n' < "$VAULT_PUBKEY_FILE")
export VAULT_UPDATER_STABLE_URL
export VAULT_UPDATER_BETA_URL

export TAURI_SIGNING_PRIVATE_KEY_PATH="$VAULT_KEY_DIR/vault-key.json"
# The Tauri CLI's bundler reads ONLY `TAURI_SIGNING_PRIVATE_KEY` — never
# the `_PATH` variant. `cargo-tauri signer sign` accepts the `_PATH` form,
# but `tauri build` does not, so exporting only `_PATH` made the operator
# release path fail with:
#
#   "A public key has been found, but no private key. Make sure to set
#    `TAURI_SIGNING_PRIVATE_KEY` environment variable."
#
# The CLI accepts either the key material or a path to the file, so the
# path is exported here. Both names are set for compatibility.
export TAURI_SIGNING_PRIVATE_KEY="$VAULT_KEY_DIR/vault-key.json"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(cat "$VAULT_KEY_PASSWORD_FILE")

# ── Echo a redacted manifest (never the private key) for the run log ──
{
  echo "── vault updater env loaded ──"
  echo "  VAULT_UPDATER_STABLE_URL       = $VAULT_UPDATER_STABLE_URL"
  echo "  VAULT_UPDATER_BETA_URL         = $VAULT_UPDATER_BETA_URL"
  echo "  VAULT_UPDATER_PUBKEY (sha256)  = $(printf '%s' "$VAULT_UPDATER_PUBKEY" | sha256sum | cut -c1-16)…"
  echo "  TAURI_SIGNING_PRIVATE_KEY_PATH = $TAURI_SIGNING_PRIVATE_KEY_PATH (mode $priv_mode)"
  echo "  TAURI_SIGNING_PRIVATE_KEY_PASSWORD = <redacted, length ${#TAURI_SIGNING_PRIVATE_KEY_PASSWORD}>"
  echo "  public key (first 40 chars)    = ${VAULT_UPDATER_PUBKEY:0:40}…"
} >&2
