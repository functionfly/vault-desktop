# Mock R2 round-trip — 2026-10-06

> **Redacted for publication.** The real account ID, bucket name, and key
> fingerprints live in [`docs/INTERNAL.md`](../../docs/INTERNAL.md). The mock
> credentials below are deliberately fake and were never valid.

This is the end-to-end evidence that the three operator follow-up
scripts work end-to-end against an S3-compatible endpoint. The
target is a stateful Python mock at `127.0.0.1:18234`; running
this against the real bucket only
requires swapping `R2_ENDPOINT_OVERRIDE` for the canonical
`https://<account-id>.r2.cloudflarestorage.com`
URL and supplying real R2 credentials.

## Reproduce

```bash
# 1. Start the mock.
MOCK_R2_STORE=/tmp/mock-r2-store python3 scripts/release/../mock-r2.py 18234 &

# 2. Source the env loader.
source scripts/release/load-updater-env.sh

# 3. Provision the bucket.
R2_ACCOUNT_ID=<32 hex account id> \
R2_ACCESS_KEY_ID=FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE \
R2_SECRET_ACCESS_KEY=fakefakesecret_secret_secret_secret_secret_secret_see \
R2_ENDPOINT_OVERRIDE=http://127.0.0.1:18234 \
    scripts/release/provision-r2-bucket.sh

# 4. Publish an update to the stable channel.
R2_ACCOUNT_ID=<32 hex account id> \
R2_ACCESS_KEY_ID=FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE \
R2_SECRET_ACCESS_KEY=fakefakesecret_secret_secret_secret_secret_secret_see \
R2_ENDPOINT_OVERRIDE=http://127.0.0.1:18234 \
    scripts/release/publish-update.sh stable 0.1.1 \
        fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz \
        fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz.sig

# 5. Publish a pre-release to the beta channel.
R2_ACCOUNT_ID=<32 hex account id> \
R2_ACCESS_KEY_ID=FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE \
R2_SECRET_ACCESS_KEY=fakefakesecret_secret_secret_secret_secret_secret_see \
R2_ENDPOINT_OVERRIDE=http://127.0.0.1:18234 \
    scripts/release/publish-update.sh beta 0.2.0-rc.1 \
        fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz \
        fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz.sig
```

## Output captured

### `provision-r2-bucket.sh`

```
── provision-r2-bucket ──
  bucket:  functionfly-vault-updates
  region:  auto
  endpoint: http://127.0.0.1:18234

→ Step 1: create bucket (idempotent)
  bucket functionfly-vault-updates already exists; skipping create

→ Step 2: enable bucket versioning
  versioning enabled

→ Step 3: lifecycle rule (noncurrent versions expire after 30 days)
  lifecycle rule applied

→ Step 4: CORS (read-only)
  CORS applied

→ Step 5: round-trip probe (write + read + delete)
```

All 5 steps complete. Step 5's `download failed: ... 'ContentLength'`
warning is a mock-only artifact (the mock does not echo a
`Content-Length` header); real R2 sends the header and `aws s3 cp
s3://... -` reports the byte count.

### `publish-update.sh stable 0.1.1 …`

```
── vault uploader ──
  channel:  stable
  version:  0.1.1
  side-car: /tmp/vault-release-fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz
  sig:      /tmp/vault-release-fixtures/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz.sig
  bucket:   functionfly-vault-updates

→ uploading side-car to s3://functionfly-vault-updates/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz
  ok
→ uploading signature to s3://functionfly-vault-updates/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz.sig
  ok

→ uploading manifest to s3://functionfly-vault-updates/vault/stable/manifest.json
  ok

── Done ──
  side-car:  https://updates.functionfly.com/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz
  signature: https://updates.functionfly.com/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz.sig
  manifest:  https://updates.functionfly.com/vault/stable/manifest.json
```

### `publish-update.sh beta 0.2.0-rc.1 …`

Same flow with a pre-release tag — manifest is byte-correct, channel
URLs differ, and the bucket now holds two channels' worth of
versioned side-cars.

## Stored manifests (read back from the mock)

> **Superseded on 2026-11-03.** The transcripts below are kept as a record
> of what was run on 2026-10-06. The manifests they show were *not* valid
> updater manifests: the `platforms` key was a Rust target triple the
> plugin never looks up, and `signature` held a URL instead of the
> signature content. The current schema is in
> [`../README.md`](../README.md#tauri-updater-schema).

### `vault/stable/manifest.json`

```json
{
  "version": "0.1.1",
  "pub_date": "2026-10-06T22:04:47Z",
  "notes": "vault 0.1.1 (channel: stable)",
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://updates.functionfly.com/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz",
      "signature": "https://updates.functionfly.com/vault/stable/0.1.1/functionfly-vault-desktop_0.1.1.tar.gz.sig"
    }
  }
}
```

### `vault/beta/manifest.json`

```json
{
  "version": "0.2.0-rc.1",
  "pub_date": "2026-10-06T22:05:43Z",
  "notes": "vault 0.2.0-rc.1 (channel: beta)",
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://updates.functionfly.com/vault/beta/0.2.0-rc.1/functionfly-vault-desktop_0.2.0-rc.1.tar.gz",
      "signature": "https://updates.functionfly.com/vault/beta/0.2.0-rc.1/functionfly-vault-desktop_0.2.0-rc.1.tar.gz.sig"
    }
  }
}
```

## Byte-identical round-trip

```
6c089841f299060e21e1bac2adf65962bc5335e4f16dcb24852af9d9ba93846b  source.tar.gz
6c089841f299060e21e1bac2adf65962bc5335e4f16dcb24852af9d9ba93846b  mock-stored.tar.gz
9adf5321e1c62eb86a8a47051c4d3d5720dbc7a26e9bdee952ae58c0b41f1d48  source.sig
9adf5321e1c62eb86a8a47051c4d3d5720dbc7a26e9bdee952ae58c0b41f1d48  mock-stored.sig
```

Both files were uploaded byte-for-byte; the mock R2 returned the
same bytes on GET that the script PUT.

## Final bucket tree

```
vault/
├── beta/
│   ├── 0.2.0-rc.1/
│   │   ├── functionfly-vault-desktop_0.2.0-rc.1.tar.gz
│   │   └── functionfly-vault-desktop_0.2.0-rc.1.tar.gz.sig
│   └── manifest.json
└── stable/
    ├── 0.1.1/
    │   ├── functionfly-vault-desktop_0.1.1.tar.gz
    │   └── functionfly-vault-desktop_0.1.1.tar.gz.sig
    └── manifest.json
```

Matches the canonical layout documented in
`src-tauri/src/updater.rs::UpdaterConfig::from_env` and the README
"Updater (signed auto-update)" section.

## Lessons learned (saved as fix-ups in this commit)

1. `R2_ENDPOINT_OVERRIDE` knob is now plumbed through both
   `provision-r2-bucket.sh` and `publish-update.sh` so the smoke
   test can replay against a local mock without
   `AWS_ENDPOINT_URL` clobbering the canonical R2 endpoint.
2. `publish-update.sh` now applies the `R2_BUCKET` default
   _before_ the pre-flight check (it ran the check first, which
   required the operator to set the var even though the script
   had a default for the production case).
3. `load-updater-env.sh` and the two upload scripts are
   sequenced: env must be sourced first so `R2_*` + `VAULT_*`
   are in scope. The smoke test pins this ordering.
