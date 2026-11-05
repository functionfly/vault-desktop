#!/usr/bin/env bash
# Verifies that the updater public key committed to `tauri.conf.json` and
# the private key used to sign release artefacts are the SAME keypair.
#
# WHY THIS EXISTS
# ---------------
# The Tauri CLI signs the updater artefacts with `TAURI_SIGNING_PRIVATE_KEY`
# but reads the *verification* key from `plugins.updater.pubkey` in
# `tauri.conf.json`. If those two ever drift apart, the release still
# builds and still publishes — the failure only surfaces on a user's
# machine, where every update fails signature verification.
#
# The CLI detects the drift itself and prints a warning
# ("The updater secret key ... does not match the public key"), but a
# warning in a build log is not a gate. This script turns it into a
# hard failure, and it runs in CI.
#
# WHAT IT CHECKS
# --------------
# minisign encodes a key as:
#
#   base64( "untrusted comment: ...\n"
#           base64( alg[2] | keynum[8] | payload ) )
#
# The `keynum` is the 8 bytes that identify the key. This script decodes
# the public key from `tauri.conf.json` and the public key derived from
# the private key, and asserts the keynums are equal.
#
# USAGE
# -----
#   VAULT_SIGNING_PRIVATE_KEY_PATH=/path/to/vault-key.json \
#   VAULT_SIGNING_PRIVATE_KEY_PASSWORD=... \
#     ./scripts/release/verify-updater-keypair.sh
#
# Without the private key the script verifies what it can (that the
# configured pubkey is a well-formed minisign public key) and reports
# that the keypair match was NOT verified, rather than passing silently.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

TAURI_CONF="${TAURI_CONF:-src-tauri/tauri.conf.json}"

fail() {
  echo "::error::$1" >&2
  exit 1
}

command -v python3 >/dev/null 2>&1 || fail "python3 is required"

[ -f "$TAURI_CONF" ] || fail "$TAURI_CONF not found"

# ── The configured public key must exist and be well-formed ───────────────
PUBKEY="$(python3 - "$TAURI_CONF" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
section = cfg.get("plugins", {}).get("updater", {})
pubkey = section.get("pubkey", "")
if not pubkey.strip():
    sys.stderr.write(
        "plugins.updater.pubkey is empty in " + sys.argv[1] + "\n"
        "The updater would ship with no verification key: the Tauri CLI\n"
        "cannot sign the updater artefacts and installed clients cannot\n"
        "verify them.\n"
    )
    raise SystemExit(1)
print(pubkey)
PY
)" || fail "the configured updater public key is missing or empty"

python3 - "$PUBKEY" <<'PY'
import base64, sys
text = base64.b64decode(sys.argv[1].strip()).decode()
lines = text.split("\n")
body = base64.b64decode(lines[1])
if len(body) < 10:
    sys.stderr.write("the configured pubkey is not a minisign key box\n")
    raise SystemExit(1)
print("configured pubkey keynum:", body[2:10].hex().upper())
PY

# ── Cross-check against the signing key when it is available ──────────────
# The private key can arrive under several names depending on how the
# build was invoked:
#   * `VAULT_SIGNING_PRIVATE_KEY_PATH` / `VAULT_SIGNING_PRIVATE_KEY` —
#     the names this script's own docs use, and the ones the release
#     workflow sets.
#   * `TAURI_SIGNING_PRIVATE_KEY_PATH` / `TAURI_SIGNING_PRIVATE_KEY` —
#     the names the Tauri CLI itself reads, and the ones
#     `load-updater-env.sh` exports.
#
# All four are accepted so that sourcing the env loader and then running
# this script finds the key. Both are normalised to a file on disk,
# because `cargo-tauri signer sign` takes a path.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

KEY_PATH=""
for candidate in \
  "${VAULT_SIGNING_PRIVATE_KEY_PATH:-}" \
  "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}"
do
  if [ -n "$candidate" ] && [ -f "$candidate" ]; then
    KEY_PATH="$candidate"
    break
  fi
done

if [ -z "$KEY_PATH" ]; then
  for material in \
    "${VAULT_SIGNING_PRIVATE_KEY:-}" \
    "${TAURI_SIGNING_PRIVATE_KEY:-}"
  do
    if [ -n "$material" ]; then
      KEY_PATH="$TMP/private-key.json"
      # A trailing newline is not part of the key and breaks base64
      # decoding, so strip it.
      printf '%s' "$material" > "$KEY_PATH"
      chmod 600 "$KEY_PATH"
      break
    fi
  done
fi

if [ -z "$KEY_PATH" ]; then
  echo "::notice::no signing key was provided; the keypair match was NOT verified."
  echo "         Only the presence and well-formedness of the configured pubkey were checked."
  exit 0
fi

if [ -z "${VAULT_SIGNING_PRIVATE_KEY_PASSWORD:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
  echo "::notice::the signing key password is not set; the keypair match was NOT verified."
  exit 0
fi
SIGNING_PASSWORD="${VAULT_SIGNING_PRIVATE_KEY_PASSWORD:-${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}}"

command -v cargo-tauri >/dev/null 2>&1 || {
  # `cargo install` puts the binary in CARGO_HOME/bin, which is not always
  # on a non-interactive shell's PATH.
  CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
  if [ -x "$CARGO_BIN_DIR/cargo-tauri" ]; then
    PATH="$CARGO_BIN_DIR:$PATH"
    export PATH
  else
    echo "::notice::cargo-tauri is not installed; the keypair match was NOT verified."
    exit 0
  fi
}

# Sign a throwaway file with the real private key, then read the keynum
# out of the produced signature.
#
# `cargo-tauri signer sign` maps the TAURI_SIGNING_PRIVATE_KEY environment
# variable onto its `--private-key` argument. Passing `--private-key-path`
# as well is a clap conflict ("the argument '--private-key-path' cannot be
# used with '--private-key'"), and the error is easy to miss because the
# signing step looks like a plain failure. The env vars are cleared for
# this one invocation and everything is passed explicitly.
head -c 4096 /dev/urandom > "$TMP/probe.bin"
env -u TAURI_SIGNING_PRIVATE_KEY \
    -u TAURI_SIGNING_PRIVATE_KEY_PATH \
    cargo-tauri signer sign \
      --private-key-path "$KEY_PATH" \
      --password "$SIGNING_PASSWORD" \
      "$TMP/probe.bin" >/dev/null 2>&1 \
  || fail "could not sign a probe artefact with $KEY_PATH"

python3 - "$PUBKEY" "$TMP/probe.bin.sig" <<'PY'
import base64, json, sys

def parse_box(b64_text):
    text = base64.b64decode(b64_text.strip()).decode()
    body = base64.b64decode(text.split("\n")[1])
    return body[2:10], body[10:]

configured_keynum, configured_pk = parse_box(sys.argv[1])
signature_keynum, _ = parse_box(open(sys.argv[2]).read())

print("configured pubkey keynum:", configured_keynum.hex().upper())
print("signing key keynum       :", signature_keynum.hex().upper())

if configured_keynum != signature_keynum:
    sys.stderr.write(
        "\nThe updater public key in tauri.conf.json does NOT match the "
        "signing key.\nA release built this way would publish updater "
        "artefacts that no client\ncan verify.\n"
    )
    raise SystemExit(1)

print("\nKeypair matches: release artefacts signed with this key will be "
      "accepted by clients built from this tree.")
PY
