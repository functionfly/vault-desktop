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
#   VAULT_UPDATER_PUBLIC_HOST — the public hostname the bucket is served
#                           from (default: updates.functionfly.com). The
#                           custom domain is attached to the bucket in the
#                           same Cloudflare ACCOUNT as the DNS zone.
#   CF_ZONE_ID            — the Cloudflare zone id for that hostname
#                           (required to attach the custom domain)
#   CLOUDFLARE_API_TOKEN  — a Cloudflare API token with R2 write scope,
#                           used to attach the custom domain. Objects are
#                           still uploaded with the R2 credentials above;
#                           this token is only needed for the domain.
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
# The public hostname the bucket is served from. `publish-update.sh` writes
# this same host into the manifest's `url` fields, so the two must agree:
# a manifest pointing at a hostname the bucket is not attached to serves
# URLs that 404 for every client.
VAULT_UPDATER_PUBLIC_HOST="${VAULT_UPDATER_PUBLIC_HOST:-updates.functionfly.com}"
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

# 5. Attach the public custom domain.
#
# WHY THIS STEP EXISTS
# -------------------
# Without it the bucket is reachable ONLY through the S3 API, which needs
# credentials. The updater client has none: it fetches
# `https://<host>/vault/<channel>/manifest.json` anonymously and then
# downloads the artefact from the URL in that manifest. A bucket with no
# custom domain attached answers such a request with a connection reset or
# a hang — the DNS name resolves (it is a proxied Cloudflare record) but
# no origin answers it. That is exactly the failure this step prevents:
# a published manifest and a manifest URL that 404s for every client.
#
# The domain must live in the SAME Cloudflare account as the DNS zone,
# or Cloudflare refuses the attachment.
#
# The attachment is idempotent: attaching an already-attached domain is a
# no-op, so this script stays safe to re-run.
echo
echo "→ Step 5: attach the public custom domain (${VAULT_UPDATER_PUBLIC_HOST})"
if [[ -z "${CF_ZONE_ID:-}" || -z "${CLOUDFLARE_API_TOKEN:-}" ]]; then
  echo "  SKIPPED: CF_ZONE_ID and CLOUDFLARE_API_TOKEN are not both set." >&2
  echo "  Without a custom domain the updater CANNOT fetch the manifest:" >&2
  echo "  the bucket is then reachable only with credentials, which the" >&2
  echo "  updater client does not have." >&2
  echo "  Set both and re-run, or attach the domain by hand:" >&2
  echo "    wrangler r2 bucket domain add ${R2_BUCKET} \\" >&2
  echo "      --domain ${VAULT_UPDATER_PUBLIC_HOST} --zone-id <zone-id>" >&2
else
  DOMAIN_STATUS="$(curl -sS \
    -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
    "https://api.cloudflare.com/client/v4/accounts/${R2_ACCOUNT_ID}/r2/buckets/${R2_BUCKET}/custom_domains" \
    2>/dev/null || true)"

  if printf '%s' "$DOMAIN_STATUS" | grep -q "\"domain\":\"${VAULT_UPDATER_PUBLIC_HOST}\""; then
    echo "  already attached"
  else
    ATTACH_RESPONSE="$(curl -sS -X POST \
      -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
      -H "Content-Type: application/json" \
      -d "{\"hostname\":\"${VAULT_UPDATER_PUBLIC_HOST}\",\"zone_id\":\"${CF_ZONE_ID}\"}" \
      "https://api.cloudflare.com/client/v4/accounts/${R2_ACCOUNT_ID}/r2/buckets/${R2_BUCKET}/custom_domains" \
      2>/dev/null || true)"

    if printf '%s' "$ATTACH_RESPONSE" | grep -q '"success":true'; then
      echo "  attached (SSL issuance and ownership validation take a few minutes)"
    else
      echo "  FAILED to attach ${VAULT_UPDATER_PUBLIC_HOST}:" >&2
      printf '%s\n' "$ATTACH_RESPONSE" >&2
      echo "  The updater will not be able to fetch the manifest until this" >&2
      echo "  succeeds. Attach it with:" >&2
      echo "    wrangler r2 bucket domain add ${R2_BUCKET} \\" >&2
      echo "      --domain ${VAULT_UPDATER_PUBLIC_HOST} --zone-id ${CF_ZONE_ID}" >&2
      exit 1
    fi
  fi
fi

# 6. Smoke-check the bucket via a write + read round-trip.
echo
echo "→ Step 6: round-trip probe (write + read + delete)"
PROBE_KEY="vault/_probe-$(date +%s)"
echo "probe-payload-$(date +%s)" | aws s3 cp - "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
aws s3 cp "s3://${R2_BUCKET}/${PROBE_KEY}" - --endpoint-url "$ENDPOINT" >/dev/null
aws s3 rm "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
echo "  probe OK"

# 7. Prove the PUBLIC path works, not just the credentialed one.
#
# The round-trip above proves the bucket is writable and readable BY THE
# OPERATOR. It says nothing about the updater client, which has no
# credentials. This probe fetches over the public hostname with no auth
# and no cookie jar, exactly as an installed client would.
echo
echo "→ Step 7: anonymous public-reachability probe"
PROBE_KEY="vault/_probe-public-$(date +%s)"
echo "probe-payload-$(date +%s)" | aws s3 cp - "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
PUBLIC_CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 25 \
  "https://${VAULT_UPDATER_PUBLIC_HOST}/${PROBE_KEY}" 2>/dev/null || echo 000)"
aws s3 rm "s3://${R2_BUCKET}/${PROBE_KEY}" --endpoint-url "$ENDPOINT" >/dev/null
if [[ "$PUBLIC_CODE" = "200" ]]; then
  echo "  anonymous GET https://${VAULT_UPDATER_PUBLIC_HOST}/${PROBE_KEY} -> 200 OK"
else
  echo "  FAILED: anonymous GET https://${VAULT_UPDATER_PUBLIC_HOST}/${PROBE_KEY} -> ${PUBLIC_CODE}" >&2
  echo "  The bucket is not publicly reachable. An installed client would" >&2
  echo "  fetch the manifest and receive this same failure." >&2
  exit 1
fi

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
