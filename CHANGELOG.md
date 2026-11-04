# Changelog

All notable changes to FunctionFly Vault Desktop are recorded here. The
project follows [Semantic Versioning](https://semver.org/).

## [0.1.0] — 2026-11-03 — first public release

### Highlights

- **FunctionFly Vault, on macOS, Linux, and Windows.** A standalone
  desktop client built with Tauri v2 + React 19. Login with email+password
  or an API key, unlock the vault with a passphrase, browse + reveal +
  rotate + rollback secrets.
- **Byte-exact zero-knowledge crypto.** Every client-side encrypt /
  decrypt uses PBKDF2-HMAC-SHA256 (600 000 iterations for v2, 100 000
  for v1) + AES-256-GCM with 12-byte nonce + 16-byte tag. Same
  `EncryptedData` shape as the dashboard (`web/dashboard/src/utils/vault-crypto.ts`)
  and the Go SDK (`sdk/go-vault-sdk/vault/crypto.go`) — secrets created
  in any one of the three are round-trip readable in the others.
- **No bearer token in the webview.** Every `/v1/vault/*` call originates
  in the Rust core, with the access + refresh tokens held in the OS
  keychain (`functionfly-vault-desktop` service on Linux libsecret /
  Apple Keychain / Windows Credential Manager). The React UI talks to the
  Rust core over typed Tauri commands; the IPC surface is mocked in
  the test suite and rejects the webview when the Tauri host is
  absent.
- **CSRF on every mutation.** POST / PATCH / DELETE calls fetch a
  one-time `/v1/csrf` token and send it as `X-CSRF-Token`. The
  HTTP client retries once on 403 with body `csrf_token_invalid`.
- **Auto-locks on close.** The passphrase is wrapped in a
  `Zeroizing<String>` mutex and wiped on `WindowEvent::CloseRequested`.

### IPC surface (15 core commands + 5 mutating helpers + 8 launch extras)

```
login, login_with_api_key, restore_session, logout,
unlock_vault, lock_vault, vault_status,
list_secrets, get_secret, list_secret_versions,
reveal_secret, diff_secret_versions, rollback_secret,
list_namespaces, list_audit
```

Plus 5 mutating helpers wired through the same envelope:
`create_secret`, `update_secret`, `rotate_secret`, `delete_secret`.

Launch extras (added 2026-10-06 to round out the wider vault feature set):

```
list_tokens, generate_token, revoke_token,
bulk_delete_secrets,
create_namespace, delete_namespace,
list_audit_by_action, list_audit_by_actor
```

### Tests

- 52 Rust unit tests (`cargo test --lib`) — AES-256-GCM roundtrip,
- 19 mockito-backed integration tests
  (`cargo test --test orchestrator_mock`) — login round-trip, CSRF
  fetch + retry, non-loopback base-URL rejection, fail-closed without
  a token, captcha-required surfacing.
- 10 device-flow integration tests (`cargo test --test device_flow`) —
  the RFC 8628 device-grant exchange.
- 4 updater-config contract tests (`cargo test --test updater_config`) —
  the verification key is present, parseable by the plugin's own config
  type, and decodes as a minisign public key.
- 58 vitest contract tests (`bunx vitest run`) — every typed wrapper
  in `src/lib/ipc.ts` sends the right (command, args) shape; typed
  `{kind, message}` rejections parse into `VaultIpcError`.
- **Total: 143/143 tests pass** (52 Rust lib + 19 mockito integration +
  10 device flow + 4 updater config + 58 vitest).

### Updater

- Signed auto-update via `tauri-plugin-updater` (mirror of the
  `studio-desktop` Phase 5 pattern). Fail-closed contract: the Rust
  core refuses to check for updates without a valid Ed25519 pubkey and
  surfaces a typed `missing_pubkey` error to the webview.
- The Ed25519 public key is **compiled into the binary** via
  `plugins.updater.pubkey` in `tauri.conf.json`, so an installed
  client can verify updates with no environment variable set.
  `VAULT_UPDATER_PUBKEY` is now an *override* for operators (key
  rotation, staging), not the primary source.
- `scripts/release/verify-updater-keypair.sh` asserts that the
  committed verification key and the release signing key are the same
  keypair. It runs in CI and in `release.yml`; without it, a mismatch
  would publish artefacts that every client rejects at install time.
- Two channels: `stable` and `beta`. Per-channel endpoint env vars
  (`VAULT_UPDATER_STABLE_URL`, `VAULT_UPDATER_BETA_URL`) default to
  the canonical R2 path layout
  `https://updates.functionfly.app/vault/{channel}/manifest.json`
  so a vanilla binary still works once the R2 bucket is provisioned.
- New IPC: `vault_check_update(channel)` returns
  `{ result: "up_to_date" }` or `{ result: "available", version,
notes, pub_date }`; `vault_apply_update(channel)` downloads,
  verifies the Ed25519 signature against the configured pubkey, and
  stages install on next restart. Typed error envelope (8 codes:
  `missing_pubkey`, `invalid_endpoint`, `missing_endpoint`,
  `unknown_channel`, `builder_error`, `check_failed`,
  `no_update_available`, `install_failed`).
- New `UpdaterScreen` (accessible via the sidebar) lets the user
  pick a channel, check, and trigger install with explicit
  "Download + verify + stage" copy.
- `tauri.conf.json` flips `createUpdaterArtifacts: true` so the
  release pipeline produces a `.tar.gz` (Linux) / `.app.tar.gz` (macOS)
  / `.nsis.zip` (Windows) update archive alongside the installer.
- Operator-side release flow is documented inline in
  `tauri.conf.json`'s `updater._comment` field — the canonical
  contract lives in
  `src-tauri/src/updater.rs::UpdaterConfig::from_env`.
- **Release scripts** under `scripts/release/` (one-time per
  environment, then per release):
  - `load-updater-env.sh` — exports `VAULT_UPDATER_PUBKEY` +
    `TAURI_SIGNING_PRIVATE_KEY_*` from `~/.tauri/vault-key{,.json,-password.txt}`;
    fail-closed if any of those is missing.
  - `provision-r2-bucket.sh` — idempotent bucket create + versioning
    - lifecycle (30-day noncurrent expiry) + CORS + write/read/delete
      round-trip probe for `functionfly-vault-updates`.
  - `publish-update.sh <channel> <version> <side-car> <sig> [target]` — uploads
    the side-car + .sig and merges that platform's entry into the channel's
    `manifest.json` at the canonical `vault/{channel}/{version}/…` layout;
    channels must be `stable` or `beta`; version must be semver.
  - `publish-update.test.sh` — smoke test covering arg parsing, the
    updater manifest contract, and the artefact→manifest-target mapping,
    plus an optional **mock-R2 round-trip** that boots the bundled
    `scripts/release/mock-r2.py`, publishes a fixture side-car, and
    asserts the three expected objects land in the mock store at
    `vault/stable/{version}/…`. The round-trip is gated on
    `MOCK_R2_STORE=<path>` + `python3` + `aws` so it stays inert in CI
    environments without the AWS CLI.
  - `scripts/release/mock-r2.py` — stateful S3-compatible mock
    (~120 LoC) used by the smoke test above. Records every PUT
    under `$MOCK_R2_STORE/<bucket>/<key>` so the assertions can
    verify byte-identical round-trips. Captured end-to-end evidence
    at `scripts/release/evidence/2026-10-06-mock-r2-round-trip.md`
    (the script uploaded both `stable/0.1.1` and `beta/0.2.0-rc.1`
    manifests, the byte hashes match the local sources).
- **Key fingerprint** recorded at
  `scripts/release/KEY-FINGERPRINT.txt` so the operator (or a future
  agent) can pin the current public key on a release without
  re-running `signer generate`. Includes the rotation procedure
  (generate v2 → update env → 30-day grace window → delete v1).

### Build

- `tsc --noEmit`: clean
- `vite build`: 276 KB JS / 83 KB gz in 11.7 s
- `cargo check --all-targets`: clean, no warnings
- `cargo tauri build` produces a Linux AppImage, macOS `.app` / `.dmg`,
  and Windows `.msi` / `.exe`. Bundle matrix runs on every push via
  `.github/workflows/build.yml`.

### Manual gates

- `scripts/staging-smoke.sh` — 13-step real-orchestrator end-to-end
  script. Idempotent (per-run `STAGING_SMOKE_NONCE`); cleans up after
  itself. CI only verifies syntax + shellcheck; a reviewer runs it
  against staging before merge.

### Branding

- `scripts/generate-icons.py` — produces 16 platform-specific icons
  (Linux PNG sizes, Windows MSI square tiles, `.ico` multi-res,
  macOS `.icns`, 1024×1024 master). Re-run when the brand mark
  changes.

### Known limitations

- **No secret-value edit in place.** Metadata (name / description /
  scopes) edits via `PATCH`; changing the encrypted value is a
  rotation (`POST …/rotate`) — matches the orchestrator's API split.
- **No deep-link / OAuth flow.** Captcha-free production requires an
  API key. The UI surfaces a "Use an API key instead" link when the
  orchestrator returns 403 captcha-required.
- **Encryption/decryption blocks the tokio worker for ~0.3–1 s per
  operation** (600 000 PBKDF2 iterations). Acceptable for
  explicit, low-frequency reveal/create/rotate actions; the next
  iteration moves the work to a dedicated blocking thread if
  parallelism becomes a real workload.

### Acknowledgements

The IPC seam, capabilities manifest, and keychain model mirror
`studio-desktop/src-tauri/` — same Tauri version, same `keyring`
crate version, same least-privilege capability shape. The crypto
contract is ported byte-for-byte from
`web/dashboard/src/utils/vault-crypto.ts` and
`sdk/go-vault-sdk/vault/crypto.go`.
