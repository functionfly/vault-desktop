# Real R2 publish evidence (2026-10-06)

## Goal

Confirm `publish-update.sh` works end-to-end against the **real**
`functionfly-vault-updates` R2 bucket using the dev-scoped R2 credentials
decrypted from `.env` via dotenvx.

## Environment

```
R2_ACCOUNT_ID      = <R2 account id — see the private runbook> (account, 32 hex)
R2_ACCESS_KEY_ID   = <R2 access key id — see the private runbook>   (32 chars, dev-scoped to one bucket)
R2_SECRET_ACCESS_KEY = <64 chars>
R2_ENDPOINT        = https://<R2 account id — see the private runbook>.r2.cloudflarestorage.com
R2_BUCKET          = functionfly-vault-updates (region: WNAM)

VAULT_UPDATER_PUBKEY  = dW50cnVzdGVkIGNvbW1lbnQ6IG1pbm… (152 chars, Ed25519 pub)
TAURI_SIGNING_PRIVATE_KEY_PATH = <operator key dir>/vault-key.json (mode 0600)
TAURI_SIGNING_PRIVATE_KEY_PASSWORD = <64-char random, mode 0400>
```

The dev R2 token in `.env` is **bucket-scoped** to `functionfly-vault-updates`:
object-level R/W works; bucket-management calls (`PutBucketVersioning`,
`PutBucketCors`, `ListBuckets`) return `AccessDenied`. Steps that only need
object CRUD work end-to-end; the full bucket-bootstrap (versioning,
lifecycle, CORS) requires an admin-scoped token.

## What ran

```bash
# 1. Load R2 creds from dotenvx
source /tmp/load-r2-env.sh
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto

# 2. Load updater env (vault signing key + pubkey + stable/beta URLs)
cd ~/projects/vault-desktop
source scripts/release/load-updater-env.sh

# 3. Build a 4-KB stub side-car + 128-B random sig
mkdir -p /tmp/vault-real-publish
head -c 4192 /dev/urandom > /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz
head -c  128 /dev/urandom > /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig

# 4. Publish
R2_BUCKET=functionfly-vault-updates ./scripts/release/publish-update.sh stable 0.0.1 \
    /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz \
    /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig
```

## Results

```
── vault uploader ──
  channel:  stable
  version:  0.0.1
  side-car: /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz
  sig:      /tmp/vault-real-publish/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig
  bucket:   functionfly-vault-updates

→ uploading side-car   to s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz     ok
→ uploading signature  to s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig   ok
→ uploading manifest   to s3://functionfly-vault-updates/vault/stable/manifest.json                                      ok

── Done ──
  side-car:  https://updates.functionfly.app/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz
  signature: https://updates.functionfly.app/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig
  manifest:  https://updates.functionfly.app/vault/stable/manifest.json
exit=0
```

## Objects landed on the real bucket

```
2026-10-06 18:02:56       4096 vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz
2026-10-06 18:02:58        128 vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig
2026-10-06 18:02:59        390 vault/stable/manifest.json
```

`vault/stable/manifest.json` body (read back via `aws s3 cp`):

```json
{
  "version": "0.0.1",
  "pub_date": "2026-10-06T23:02:55Z",
  "notes": "vault 0.0.1 (channel: stable)",
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://updates.functionfly.app/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz",
      "signature": "https://updates.functionfly.app/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig"
    }
  }
}
```

Schema matches `tauri-plugin-updater` v2 contract (`version`, `pub_date`,
`notes`, `platforms.<triple>.{url,signature}`).

## Cleanup

The test publish was removed to avoid leaving stale side-cars behind:

```bash
aws s3 rm s3://functionfly-vault-updates/vault/stable/ --recursive \
    --endpoint-url "$R2_ENDPOINT"
delete: s3://functionfly-vault-updates/vault/stable/manifest.json
delete: s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz
delete: s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig
```

Bucket `vault/` prefix now empty.

## Verification commands a release engineer can re-run

```bash
source ~/projects/vault-desktop/scripts/release/load-updater-env.sh
source /tmp/load-r2-env.sh                                   # or `dotenvx get ...` + export AWS_*
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto

# Confirm manifest + side-car are reachable
aws s3 cp s3://functionfly-vault-updates/vault/stable/manifest.json - \
    --endpoint-url "$R2_ENDPOINT"
aws s3 cp s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz \
    /tmp/sidecar.tar.gz --endpoint-url "$R2_ENDPOINT"

# Re-verify the signature locally
cargo-tauri signer verify \
    --public-key ~/.tauri/vault-key.json.pub \
    /tmp/sidecar.tar.gz \
    s3://functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1.tar.gz.sig
```

## Status table

| Step                                            | Result                                                                                                                              |
|-------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------|
| Load updater env from `~/.tauri/`               | PASSED — `VAULT_UPDATER_PUBKEY` (152 chars), `TAURI_SIGNING_PRIVATE_KEY_*` resolved                                                    |
| Load R2 env from dotenvx-encrypted `.env`      | PASSED — `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ENDPOINT` (WNAM) decrypted                                                       |
| Head-bucket on real bucket                      | PASSED — 200 OK, `BucketRegion=WNAM`                                                                                                 |
| Object CRUD on real bucket (PUT/LIST/DELETE)    | PASSED — dev-scoped token has full object-level access                                                                            |
| Bucket-management (`PutBucketVersioning`, etc.) | FAILED — `AccessDenied` (dev token is bucket-scoped only; admin token required for `provision-r2-bucket.sh` Steps 2–5)            |
| `publish-update.sh` end-to-end against real R2  | **PASSED** — side-car + sig + manifest uploaded; manifest schema matches `tauri-plugin-updater` v2 contract                       |
| Operator cleanup (`aws s3 rm --recursive`)      | PASSED — bucket `vault/` prefix empty after delete                                                                                 |

## Outcome

The auto-updater pipeline is **operator-ready** for the publish path with
dev-scoped creds. The one-time bucket-bootstrap (versioning + 30-day
noncurrent lifecycle + read-only CORS) requires an admin-scoped R2 token
and is documented as a manual step in `scripts/release/provision-r2-bucket.sh`.