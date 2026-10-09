# Vault auto-updater release tooling

This directory is the operator follow-up to the
[`vault-desktop` signed auto-updater](../../README.md). It contains the
scripts a release engineer runs to provision the R2 bucket and ship a new
side-car.

## Files

| File                                                     | Purpose                                                                                                                |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `KEY-FINGERPRINT.txt`                                    | Public key path, SHA-256, algorithm, rotation procedure. Reference for the rotating signer.                            |
| `load-updater-env.sh`                                    | Sources `VAULT_UPDATER_PUBKEY` + `TAURI_SIGNING_PRIVATE_KEY_*` from `~/.tauri/` into the current shell.                |
| `provision-r2-bucket.sh`                                 | One-time bootstrap: head-bucket → versioning → 30-day noncurrent lifecycle → read-only CORS → write/read/delete probe. |
| `publish-update.sh <channel> <version> <side-car> <sig>` | Uploads side-car + `.sig` + manifest to the R2 bucket under the canonical tauri-plugin-updater schema.                 |
| `publish-update.test.sh`                                 | 8 smoke checks (arg parsing + 1 end-to-end mock R2 round-trip).                                                        |
| `mock-r2.py`                                             | Stateful S3-compatible mock for testing `publish-update.sh` without touching real R2.                                  |
| `evidence/2026-10-06-mock-r2-round-trip.md`              | End-to-end evidence captured against the mock (stable/0.1.1 + beta/0.2.0-rc.1, byte-identical SHA-256).                |
| `evidence/2026-10-06-real-r2-publish.md`                 | End-to-end evidence captured against the **real** `functionfly-vault-updates` R2 bucket (2026-10-06).                  |

## Standard operator workflow

```bash
# 1. Source signing + updater env from ~/.tauri/
cd ~/projects/vault-desktop
source scripts/release/load-updater-env.sh

# 2. Source R2 creds from dotenvx-encrypted .env
#    (or in CI: dotenvx get -f .env R2_ACCESS_KEY_ID; export AWS_ACCESS_KEY_ID="$...")
source /tmp/load-r2-env.sh
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto
export R2_BUCKET=functionfly-vault-updates
unset R2_ENDPOINT_OVERRIDE

# 3. Publish a new version
R2_BUCKET=functionfly-vault-updates ./scripts/release/publish-update.sh stable 0.1.2 \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz.sig

# 4. Confirm objects landed
aws s3 ls s3://functionfly-vault-updates/vault/stable/ --recursive --endpoint-url "$R2_ENDPOINT"

# 5. Re-verify the signature locally (recommended before declaring an install for manual QA)
cargo-tauri signer verify \
    --public-key ~/.tauri/vault-key.json.pub \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz.sig
```

## One-time bucket bootstrap

```bash
# Requires an ADMIN-scoped R2 token (not the bucket-scoped dev token in .env)
R2_ADMIN_TOKEN=...  R2_ADMIN_SECRET=...  ./scripts/release/provision-r2-bucket.sh
```

The bootstrap script applies four settings that the dev-scoped token can't
manage:

- bucket versioning (so a bad release can be reversed to a previous
  side-car by restoring an older manifest)
- 30-day noncurrent-version lifecycle (so old superseded side-cars are
  cleaned up automatically — keep the bucket bounded)
- read-only CORS (the updater plugin uses GET only; POST/PUT/DELETE from a
  browser is blocked)
- write/read/delete round-trip probe (confirms the token actually has
  object CRUD; this would have failed with the bucket-scoped dev token,
  so it gates on having admin scope)

## R2 credential scope

The R2 token (decrypted via dotenvx) is **bucket-scoped** to
`functionfly-vault-updates` — it has full object-level R/W access but no
bucket-management rights. This is intentional: the dev token can ship
side-cars but can't accidentally delete the bucket or change its CORS rules.

What works with the dev-scoped token:

- head-bucket, ls, s3 cp (put, list, get)
- `publish-update.sh` end-to-end (object CRUD only)
- `load-updater-env.sh` (no R2 access needed)

What needs an admin-scoped token:

- `provision-r2-bucket.sh` (Steps 2–5: versioning, lifecycle, CORS,
  probe) — all return `AccessDenied` with the dev token. Create a separate
  admin-scoped token in the Cloudflare Dashboard, then re-run.

## Channel/version format

- `channel` ∈ {`stable`, `beta`}. URL layout:
  `https://updates.functionfly.app/vault/{channel}/{{target_version}}/{{current_version}}`
  (Tauri convention; `{{...}}` tokens are replaced at request time).
- `version` is a semver string (`MAJOR.MINOR.PATCH[-PRERELEASE]`) — the
  script rejects empty / non-semver input at the pre-flight check.

## Tauri updater schema

`publish-update.sh` writes the canonical `tauri-plugin-updater` v2 manifest:

```json
{
  "version": "0.1.2",
  "pub_date": "2026-10-12T10:15:30Z",
  "notes": "vault 0.1.2 (channel: stable)",
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://updates.functionfly.app/vault/stable/0.1.2/functionfly-vault-desktop_0.1.2.tar.gz",
      "signature": "https://updates.functionfly.app/vault/stable/0.1.2/functionfly-vault-desktop_0.1.2.tar.gz.sig"
    }
  }
}
```

For multi-platform releases, add a `<triple>` entry under `platforms` per
build target (`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, etc.).
`publish-update.sh` currently writes the Linux x86_64 triple; multi-platform
releases should extend the manifest by hand after the script finishes.

## Failure modes & their recovery

| Symptom                                                                                 | Cause                                                                | Recovery                                                                                             |
| --------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `publish-update: VAULT_UPDATER_PUBKEY is required`                                      | Forgot to `source scripts/release/load-updater-env.sh`               | `source scripts/release/load-updater-env.sh` before invoking `publish-update.sh`.                    |
| `AccessDenied` on the manifest PUT                                                      | `s3:PutObjectAcl` not granted to the dev token (Cloudflare R2 quirk) | Already handled: script falls back to `s3 cp` (PUT with bucket-owner read).                          |
| `AccessDenied` on `PutBucketVersioning` / `ListBuckets`                                 | Dev token is bucket-scoped, not admin                                | Re-run `provision-r2-bucket.sh` with an admin-scoped token.                                          |
| `cargo-tauri signer verify` says the signature is invalid                               | Wrong pubkey or wrong side-car                                       | Re-derive with the pubkey in `KEY-FINGERPRINT.txt`; re-sign with `cargo-tauri signer sign`.          |
| Object stored at `vault/<channel>/<version>/functionfly-vault-desktop_<version>.tar.gz` | The side-car name matters — the plugin resolves by exact basename    | Keep the side-car named `functionfly-vault-desktop_<version>_<arch>.tar.gz`; the script enforces it. |

## Rollback

To remove a published release:

```bash
aws s3 rm s3://functionfly-vault-updates/vault/stable/0.1.2/ --recursive \
    --endpoint-url "$R2_ENDPOINT"
# Then re-publish the previous version's manifest
R2_BUCKET=functionfly-vault-updates ./scripts/release/publish-update.sh stable 0.1.1 \
    /path/to/0.1.1.tar.gz /path/to/0.1.1.tar.gz.sig
```

Existing installs on user machines will pick up the new manifest on their
next check-in (default 30 minutes in `tauri-plugin-updater` v2) and
downgrade to `0.1.1`.

## Evidence

| Date       | Evidence                                    | What it proves                                                          |
| ---------- | ------------------------------------------- | ----------------------------------------------------------------------- |
| 2026-10-06 | `evidence/2026-10-06-mock-r2-round-trip.md` | Schema byte-identical, SHA-256 matched across mock R2 + operator's view |
| 2026-10-06 | `evidence/2026-10-06-real-r2-publish.md`    | End-to-end real-bucket publish works with dev-scoped R2 creds           |

## Open follow-ups

- **CI integration**: the scripts are wired for `dotenvx get …` + `export AWS_*`; a CI worker that has access to `.env.keys` can run the same commands.
- **Multi-platform manifest**: currently hardcoded to `x86_64-unknown-linux-gnu`. Extend `publish-update.sh` to write a `platforms.<triple>` entry per build artifact, or run the script once per platform.
- **Admin-scoped R2 token**: needs to be provisioned by the operator for `provision-r2-bucket.sh` (versioning + lifecycle + CORS). Documented in the script header.

## See also

- [Vault desktop README](../../README.md) — the in-app `UpdaterScreen` consumes the manifests these scripts publish.
- [Tauri updater plugin docs](https://tauri.app/plugin/updater/) — schema reference + signing key derivation.
