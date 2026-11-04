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
| `verify-updater-keypair.sh`                              | Asserts the verification key in `tauri.conf.json` and the release signing key are the same keypair. Fails the release if they drift. |
| `provision-r2-bucket.sh`                                 | One-time bootstrap: head-bucket → versioning → 30-day noncurrent lifecycle → read-only CORS → public custom domain → write/read/delete probe → anonymous public-reachability probe. |
| `publish-update.sh <channel> <version> <side-car> <sig> [target]` | Uploads one side-car + `.sig` and merges that platform's entry into the channel manifest.                    |
| `publish-updater-assets.sh <channel> <version> <dir>`   | Publishes every updater artefact in a directory, building one manifest across all platforms.                             |
| `render-manifest.py`                                     | The manifest contract, in one place. Emits valid JSON and refuses a target triple, a non-base64 signature, or a repointed platform. |
| `updater-targets.py`                                     | Maps an artefact filename to the manifest target keys the plugin looks up.                                               |
| `publish-update.test.sh`                                 | Smoke checks: arg parsing, the manifest contract, the artefact→target mapping, and an optional mock-R2 round-trip.       |
| `mock-r2.py`                                             | Stateful S3-compatible mock for testing `publish-update.sh` without touching real R2.                                  |
| `evidence/2026-10-06-mock-r2-round-trip.md`              | End-to-end evidence captured against the mock (stable/0.1.1 + beta/0.2.0-rc.1, byte-identical SHA-256).                |
| `evidence/2026-10-06-real-r2-publish.md`                 | End-to-end evidence captured against the **real** R2 bucket (2026-10-06).                                                |

## Standard operator workflow

```bash
# 1. Source signing + updater env from ~/.tauri/
cd ~/projects/vault-desktop
source scripts/release/load-updater-env.sh

# 1b. Assert the verification key and the signing key are the same pair.
#     The Tauri CLI signs artefacts with TAURI_SIGNING_PRIVATE_KEY but
#     reads the verification key from plugins.updater.pubkey in
#     tauri.conf.json. If they drift apart the release still publishes,
#     and every update fails verification on a user's machine.
./scripts/release/verify-updater-keypair.sh

# 2. Source R2 creds from dotenvx-encrypted .env
#    (or in CI: dotenvx get -f .env R2_ACCESS_KEY_ID; export AWS_ACCESS_KEY_ID="$...")
source /tmp/load-r2-env.sh
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto
export R2_BUCKET=functionfly-vault-updates
unset R2_ENDPOINT_OVERRIDE

# 3. Publish a new version
#    (publish one artefact + its .sig; the platform key is derived from
#     the file name, or pass it explicitly)
R2_BUCKET=functionfly-vault-updates ./scripts/release/publish-update.sh stable 0.1.2 \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz.sig

#    …or publish every artefact at once, building one manifest that
#    covers all platforms:
R2_BUCKET=functionfly-vault-updates ./scripts/release/publish-updater-assets.sh \
    stable 0.1.2 /tmp/vault-sidecars/

# 4. Confirm objects landed
aws s3 ls s3://functionfly-vault-updates/vault/stable/ --recursive --endpoint-url "$R2_ENDPOINT"

# 5. Re-verify the signature locally (recommended before declaring an install for manual QA)
cargo-tauri signer verify \
    --public-key ~/.tauri/vault-key.json.pub \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz \
    /tmp/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz.sig
```

### Where the verification key lives

The Ed25519 public key is compiled into the binary from
`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. That is what
makes auto-update work for an ordinary user, who has no environment
variables set. `VAULT_UPDATER_PUBKEY` is an **override** for operators
(key rotation, staging) and takes precedence over the compiled-in key.

Rotating the key therefore means updating `tauri.conf.json` as well as
the keypair on the build host — `verify-updater-keypair.sh` is what
catches it if you forget.

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

## The public custom domain

The bucket must be served from a hostname the updater can reach
**anonymously**. It cannot use the S3 API: an installed client has no
credentials, it just fetches
`https://updates.functionfly.com/vault/<channel>/manifest.json`.

So `provision-r2-bucket.sh` attaches `updates.functionfly.com` as an R2
custom domain (Step 5) and then probes the public path with an anonymous
GET (Step 7). Two things to know:

- **The domain must be attached in the same Cloudflare account as the DNS
  zone.** The bucket is in account `<R2 account id — see the private runbook>`,
  which owns `functionfly.com`. Attaching from any other account is
  refused by Cloudflare.
- **Attaching it needs a Cloudflare API token with R2 write scope**
  (`CLOUDFLARE_API_TOKEN` + `CF_ZONE_ID`), separate from the object-level
  R2 credentials. Without them the script skips the step with a loud
  warning rather than pretending the bucket is reachable.

The host is the single variable `VAULT_UPDATER_PUBLIC_HOST`, used by both
the provisioning script and the manifest's `url` fields. They must agree:
a manifest whose URLs name a hostname the bucket is not attached to serves
404s to every client while looking perfectly healthy in the bucket.

### Repairing a manifest published against the wrong host

`render-manifest.py` refuses to repoint a target that already has a URL
for the same version — repointing a LIVE version would serve different
bytes to clients that already cached the manifest.

If the published manifest named the wrong host (and was therefore never
reachable), repair it deliberately:

```bash
VAULT_UPDATER_REPOINT=true \
R2_ACCOUNT_ID=… R2_ACCESS_KEY_ID=… R2_SECRET_ACCESS_KEY=… \
  ./scripts/release/publish-update.sh beta 0.1.0 \
      ./functionfly-vault-desktop_0.1.0_amd64.AppImage \
      ./functionfly-vault-desktop_0.1.0_amd64.AppImage.sig
```

`VAULT_UPDATER_REPOINT=true` passes `--repoint` to the renderer. It is
off by default, and the release test suite asserts that it stays off.

## Channel/version format

- `channel` ∈ {`stable`, `beta`}. URL layout:
  `https://updates.functionfly.com/vault/{channel}/manifest.json`
  (the channel's static manifest — the plugin substitutes only
  `{{current_version}}`, `{{target}}`, `{{arch}}` and `{{bundle_type}}`;
  there is no `{{target_version}}` placeholder).
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
    "linux-x86_64-appimage": {
      "url": "https://updates.functionfly.com/vault/stable/0.1.2/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz",
      "signature": "<the base64 minisign signature, i.e. the content of the .sig file>"
    },
    "linux-x86_64": {
      "url": "https://updates.functionfly.com/vault/stable/0.1.2/functionfly-vault-desktop_0.1.2_amd64.AppImage.tar.gz",
      "signature": "<the same base64 minisign signature>"
    }
  }
}
```

Three properties are load-bearing:

1. **The platform key is `{os}-{arch}` or `{os}-{arch}-{installer}`** —
   `linux-x86_64`, `darwin-aarch64-app`, `windows-x86_64-nsis`. The plugin
   tries `{os}-{arch}-{bundle_type}` first and then `{os}-{arch}`. A Rust
   target triple such as `x86_64-unknown-linux-gnu` is **never** looked up,
   so a manifest keyed that way resolves to nothing on every platform.
2. **`signature` holds the signature, not a URL to it.** The plugin
   base64-decodes the field and minisign-verifies the downloaded archive
   against it. A URL here fails verification at install time.
3. **`notes` is a JSON string.** The manifest is parsed as JSON; a
   shell-quoted value produces a document no client can parse.

The bundle type is part of the key because a `.deb` user must be offered the
`.deb` archive, not the AppImage that shares its architecture.
`updater-targets.py` owns the artefact→key mapping, and
`publish-update.test.sh` asserts it.

A multi-platform release is one manifest containing every platform's entry.
`publish-update.sh` merges a platform into the channel's existing manifest
when the manifest is for the same version, and replaces it wholesale when
the version differs — a manifest for an older version must not leak its
platform entries forward, because those entries point at an older archive.

## Failure modes & their recovery

| Symptom                                                                                 | Cause                                                                | Recovery                                                                                             |
| --------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `publish-update: R2_ACCOUNT_ID is required`                                              | R2 credentials not exported                                           | Export `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`, or source the operator env.    |
| `AccessDenied` on the manifest PUT                                                      | `s3:PutObjectAcl` not granted to the dev token (Cloudflare R2 quirk) | Already handled: script falls back to `s3 cp` (PUT with bucket-owner read).                          |
| `AccessDenied` on `PutBucketVersioning` / `ListBuckets`                                 | Dev token is bucket-scoped, not admin                                | Re-run `provision-r2-bucket.sh` with an admin-scoped token.                                          |
| `cargo-tauri signer verify` says the signature is invalid                               | Wrong pubkey or wrong side-car                                       | Re-derive with the pubkey in `KEY-FINGERPRINT.txt`; re-sign with `cargo-tauri signer sign`.          |
| The client reports "no update available" but a release exists                            | The manifest's platform key does not match what the plugin looks up   | Run `updater-targets.py` on the artefact name and use its key verbatim.                               |
| The client downloads but refuses to install                                              | The manifest's `signature` is a URL instead of the signature content  | Re-publish; `render-manifest.py` embeds the `.sig` content.                                          |

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

- **CI integration**: the scripts are wired for `dotenvx get …` + `export AWS_*`; a CI worker that has access to `.env.keys` can run the same commands. `release.yml` now does this itself when the `R2_*` secrets are present.
- **Admin-scoped R2 token**: needs to be provisioned by the operator for `provision-r2-bucket.sh` (versioning + lifecycle + CORS). Documented in the script header.

## See also

- [Vault desktop README](../../README.md) — the in-app `UpdaterScreen` consumes the manifests these scripts publish.
- [Tauri updater plugin docs](https://tauri.app/plugin/updater/) — schema reference + signing key derivation.
