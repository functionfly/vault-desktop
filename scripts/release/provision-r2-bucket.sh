#!/usr/bin/env bash
# Step 3 of the vault desktop launch checklist: provision the R2
# bucket that serves the side-car update archives.
#
# Layout (matches `src-tauri/src/updater.rs::UpdaterConfig::from_env`):
#   vault/
#     stable/
#       <version>/
#         functionfly-vault-desktop_<version>_amd64.AppImage.tar.gz
#         functionfly-vault-desktop_<version>_amd64.AppImage.tar.gz.sig
#         manifest.json
#     beta/
#       <version>/…
#
# The `manifest.json` is the per-channel index the
# `tauri-plugin-updater` client fetches; see `publish-update.sh` for
# the publish side of this layout.
#
# The script is idempotent — re-running it on an already-provisioned
# bucket is a no-op (lifecycle policy is upserted, not duplicated).
#
# Required env vars:
#   R2_ACCOUNT_ID         — Cloudflare account that owns the bucket
#   R2_ACCESS_KEY_ID      — R2 API token (object read/write)
#   R2_SECRET_ACCESS_KEY  — R2 API token (object read/write)
#
# Optional:
#   R2_BUCKET             — bucket name (default: functionfly-vault-updates)
#   R2_REGION             — region (default: auto; the bucket itself is
#                           region-agnostic, but the SigV4 region must
#                           be a concrete value when calling from this
#                           script)
#   R2_LIFECYCLE_DAYS     — noncurrent-version expiry in days (default: 30)
#
# Note: this script uses the AWS S3 CLI with a custom endpoint because
# R2 ships an S3-compatible API. Install with
# `apt install awscli` (or use `pip install awscli`). The s3api
# subcommands are also reachable via curl + SigV4 if you prefer not
# to depend on awscli.

set -euo pipefail

R2_BUCKET="${R2_BUCKET:-functionfly-vault-updates}"
R2_REGION="${R2_REGION:-auto}"
R2_LIFECYCLE_DAYS="${R2_LIFECYCLE_DAYS:-30}"
# `R2_ENDPOINT_OVERRIDE` lets the smoke test (and local dev) route
# to a local mock R2 instead of the real Cloudflare endpoint. When
# unset we use the canonical R2 endpoint derived from `R2_ACCOUNT_ID`.
if [[ -n "${R2_ENDPOINT_OVERRIDE:-}" ]]; then
  ENDPOINT="$R2_ENDPOINT_OVERRIDE"
else
  ENDPOINT="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
fi

for v in R2_ACCOUNT_ID R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY; do
  if [[ -z "${!v:-}" ]]; then
    echo "provision-r2-bucket: $v is required" >&2
    exit 1
  fi
done

if ! command -v aws >/dev/null 2>&1; then
  echo "provision-r2-bucket: awscli is required (apt install awscli)" >&2
  exit 1
fi

export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION="$R2_REGION"

echo "── provision-r2-bucket ──"
echo "  bucket:  $R2_BUCKET"
echo "  region:  $R2_REGION"
echo "  endpoint: $ENDPOINT"

# 1. Create the bucket if it doesn't exist (head_bucket + create_bucket).
echo
echo "→ Step 1: create bucket (idempotent)"
if aws s3api head-bucket --bucket "$R2_BUCKET" --endpoint-url "$ENDPOINT" >/dev/null 2>&1; then
  echo "  bucket $R2_BUCKET already exists; skipping create"
else
  aws s3api create-bucket \
    --bucket "$R2_BUCKET" \
    --endpoint-url "$ENDPOINT" \
    --create-bucket-configuration LocationConstraint="$R2_REGION" \
    >/dev/null
  echo "  bucket $R2_BUCKET created"
fi

# 2. Enable versioning (so a re-publish of the same version doesn't
# silently overwrite a previously-rolled-back archive).
echo
echo "→ Step 2: enable bucket versioning"
aws s3api put-bucket-versioning \
  --bucket "$R2_BUCKET" \
  --endpoint-url "$ENDPOINT" \
  --versioning-configuration Status=Enabled
echo "  versioning enabled"

# 3. Lifecycle rule: noncurrent versions expire after
# R2_LIFECYCLE_DAYS (default 30). The current version of every
# `manifest.json` is preserved indefinitely — only the side-car
# archives (the binary blobs) get cleaned up.
echo
echo "→ Step 3: lifecycle rule (noncurrent versions expire after $R2_LIFECYCLE_DAYS days)"
LIFECYCLE_JSON=$(cat <<EOF
{
  "Rules": [
    {
      "ID": "expire-noncurrent",
      "Status": "Enabled",
      "Filter": { "Prefix": "vault/" },
      "NoncurrentVersionExpiration": { "NoncurrentDays": $R2_LIFECYCLE_DAYS }
    }
  ]
}
EOF
)
echo "$LIFECYCLE_JSON" | aws s3api put-bucket-lifecycle-configuration \
  --bucket "$R2_BUCKET" \
  --endpoint-url "$ENDPOINT" \
  --lifecycle-configuration file:///dev/stdin
echo "  lifecycle rule applied"

# 4. CORS: only `GET` is required (tauri-plugin-updater only reads).
# HEAD/OPTIONS preflight is included so Tauri's `tauri-plugin-updater`
# can pre-flight the manifest URL on some webview versions.
echo
echo "→ Step 4: CORS (read-only)"
CORS_JSON=$(cat <<'EOF'
{
  "CORSRules": [
    {
      "AllowedOrigins": ["https://functionfly.com", "app://*"],
      "AllowedMethods": ["GET", "HEAD"],
      "AllowedHeaders": ["*"],
      "ExposeHeaders": ["ETag"],
      "MaxAgeSeconds": 3000
    }
  ]
}
EOF
)
echo "$CORS_JSON" | aws s3api put-bucket-cors \
  --bucket "$R2_BUCKET" \
  --endpoint-url "$ENDPOINT" \
  --cors-configuration file:///dev/stdin
echo "  CORS applied"

# 5. Smoke-check the bucket via a write + read round-trip.
echo
echo "→ Step 5: round-trip probe (write + read + delete)"
PROBE_KEY="vault/_probe-$(date +%s)"
echo "probe-payload-$(date +%s)" | aws s3 cp - "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
aws s3 cp "s3://${R2_BUCKET}/${PROBE_KEY}" - --endpoint-url "$ENDPOINT" >/dev/null
aws s3 rm "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
echo "  probe OK"

cat <<EOF

── Done ──
The bucket is ready. To publish an update, run:

    source scripts/release/load-updater-env.sh
    ./scripts/release/publish-update.sh <channel> <version> \\
        path/to/functionfly-vault-desktop_<version>_amd64.AppImage.tar.gz \\
        path/to/functionfly-vault-desktop_<version>_amd64.AppImage.tar.gz.sig

Both archives must be produced by
\`cargo tauri build\`, which signs the side-cars automatically
when TAURI_SIGNING_PRIVATE_KEY is set (per the load-updater-env.sh
contract).
EOF
