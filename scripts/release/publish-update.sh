#!/usr/bin/env bash
# Step 4 of the vault desktop launch checklist: publish an update to
# the R2 bucket provisioned by `provision-r2-bucket.sh`.
#
# Usage (from the build host, after sourcing `load-updater-env.sh`):
#
#   ./scripts/release/publish-update.sh <channel> <version> \\
#       <side-car.tar.gz> <side-car.tar.gz.sig>
#
# Example:
#
#   ./scripts/release/publish-update.sh stable 0.1.1 \\
#       target/release/bundle/appimage/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz \\
#       target/release/bundle/appimage/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz.sig
#
# Side effects:
#   1. Uploads the side-car archive + .sig to
#      vault/<channel>/<version>/…
#   2. Computes + uploads `vault/<channel>/manifest.json` containing the
#      absolute versioned archive URLs and the per-platform entries
#      `tauri-plugin-updater` expects
#   3. Marks `manifest.json` itself as the current per-channel index
#      (it lives at vault/<channel>/manifest.json; the
#      `vault/<channel>/<version>/…` archives are the side-cars it
#      points at)
#
# Pre-flight:
#   - VAULT_UPDATER_PUBKEY              (from load-updater-env.sh)
#   - TAURI_SIGNING_PRIVATE_KEY_PATH    (from load-updater-env.sh)
#   - The side-car archive must have been produced by
#     `cargo tauri build` (which signs the .sig automatically when
#     the env vars above are set)

set -euo pipefail

usage() {
  cat >&2 <<EOF
usage: $0 <channel> <version> <side-car.tar.gz> <side-car.tar.gz.sig>
  channel:       one of: stable | beta
  version:       semver, e.g. 0.1.1
  side-car:      path to the .tar.gz archive produced by cargo tauri build
  signature:     path to the matching .sig file (produced by cargo tauri build)
EOF
  exit 64
}

# ── Arg parsing + pre-flight ───────────────────────────────────────────
[[ $# -eq 4 ]] || usage

CHANNEL="$1"
VERSION="$2"
SIDECAR_PATH="$3"
SIG_PATH="$4"

if [[ "$CHANNEL" != "stable" && "$CHANNEL" != "beta" ]]; then
  echo "publish-update: channel must be stable or beta, got: $CHANNEL" >&2
  exit 1
fi

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.-]+)?$ ]]; then
  echo "publish-update: version must be semver, got: $VERSION" >&2
  exit 1
fi

if [[ ! -s "$SIDECAR_PATH" ]]; then
  echo "publish-update: side-car not found or empty: $SIDECAR_PATH" >&2
  exit 1
fi
if [[ ! -s "$SIG_PATH" ]]; then
  echo "publish-update: signature not found or empty: $SIG_PATH" >&2
  exit 1
fi

R2_BUCKET="${R2_BUCKET:-functionfly-vault-updates}"
R2_REGION="${R2_REGION:-auto}"

for v in VAULT_UPDATER_PUBKEY TAURI_SIGNING_PRIVATE_KEY_PATH R2_ACCOUNT_ID R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_BUCKET; do
  if [[ -z "${!v:-}" ]]; then
    echo "publish-update: $v is required (run \`source scripts/release/load-updater-env.sh\` first, or export R2_* in CI)" >&2
    exit 1
  fi
done
# `R2_ENDPOINT_OVERRIDE` lets the smoke test (and local dev) route
# to a local mock R2 instead of the real Cloudflare endpoint. When
# unset we use the canonical R2 endpoint derived from `R2_ACCOUNT_ID`.
if [[ -n "${R2_ENDPOINT_OVERRIDE:-}" ]]; then
  ENDPOINT="$R2_ENDPOINT_OVERRIDE"
else
  ENDPOINT="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
fi
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION="$R2_REGION"

# ── Upload side-car + signature to the versioned path ────────────────
ARCHIVE_NAME="functionfly-vault-desktop_${VERSION}.tar.gz"
SIDECAR_KEY="vault/${CHANNEL}/${VERSION}/${ARCHIVE_NAME}"
SIG_KEY="vault/${CHANNEL}/${VERSION}/${ARCHIVE_NAME}.sig"

echo "── vault uploader ──"
echo "  channel:  $CHANNEL"
echo "  version:  $VERSION"
echo "  side-car: $SIDECAR_PATH"
echo "  sig:      $SIG_PATH"
echo "  bucket:   $R2_BUCKET"
echo

echo "→ uploading side-car to s3://${R2_BUCKET}/${SIDECAR_KEY}"
aws s3 cp "$SIDECAR_PATH" "s3://${R2_BUCKET}/${SIDECAR_KEY}" \
  --endpoint-url "$ENDPOINT" \
  --content-type application/octet-stream >/dev/null
echo "  ok"

echo "→ uploading signature to s3://${R2_BUCKET}/${SIG_KEY}"
aws s3 cp "$SIG_PATH" "s3://${R2_BUCKET}/${SIG_KEY}" \
  --endpoint-url "$ENDPOINT" \
  --content-type application/octet-stream >/dev/null
echo "  ok"

# ── Compute the manifest ──────────────────────────────────────────────
PUBLIC_BASE="https://updates.functionfly.app/vault/${CHANNEL}"
ARCHIVE_URL="${PUBLIC_BASE}/${VERSION}/${ARCHIVE_NAME}"
SIG_URL="${PUBLIC_BASE}/${VERSION}/${ARCHIVE_NAME}.sig"

# Tauri manifest schema (per tauri-plugin-updater docs):
#   {
#     "version": "0.1.1",
#     "pub_date": "<RFC3339>",
#     "platforms": { "<target-triple>": { "url": ..., "signature": ... } },
#     "notes": "..."
#   }
#
# `tauri-plugin-updater` matches the target triple at runtime; the
# canonical triple for the Linux AppImage is
# `x86_64-unknown-linux-gnu`. Operators shipping macOS / Windows
# archives extend `platforms` with the matching triples.
MANIFEST_TMP=$(mktemp)
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
NOTES="${NOTES:-vault ${VERSION} (channel: ${CHANNEL})}"

cat > "$MANIFEST_TMP" <<EOF
{
  "version": "${VERSION}",
  "pub_date": "${PUB_DATE}",
  "notes": ${NOTES@Q},
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "${ARCHIVE_URL}",
      "signature": "${SIG_URL}"
    }
  }
}
EOF

echo
echo "→ uploading manifest to s3://${R2_BUCKET}/vault/${CHANNEL}/manifest.json"
aws s3 cp "$MANIFEST_TMP" "s3://${R2_BUCKET}/vault/${CHANNEL}/manifest.json" \
  --endpoint-url "$ENDPOINT" \
  --content-type application/json >/dev/null
rm -f "$MANIFEST_TMP"
echo "  ok"

echo
echo "── Done ──"
echo "  side-car:  $ARCHIVE_URL"
echo "  signature: $SIG_URL"
echo "  manifest:  ${PUBLIC_BASE}/manifest.json"
echo
echo "Run \`cargo-tauri signer verify -p <pubkey> <side-car> <sig>\` locally"
echo "to re-verify the signature before declaring an install for manual QA."