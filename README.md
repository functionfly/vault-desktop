# FunctionFly Vault Desktop

Cross-platform desktop client for the FunctionFly zero-knowledge secrets API.
Built on Tauri v2 (Rust core + React webview) and packed into a single binary
for Windows, macOS, and Linux.

> **Status: 2026-10-06 — v0.1.0.**
> Rust core: `cargo check` clean; 31/31 unit tests + 10/10 mockito integration tests passing. React bundle: `tsc` + `vite build` clean, 28/28 vitest. Signed auto-update (Phase 5) wired behind `VAULT_UPDATER_PUBKEY` (fail-closed when unset).
> No real-staging smoke yet — that requires a live orchestrator on a host
> with a real OS keychain (the headless CI runner used for the build does not
> have one), so the manual checklist at the end of this document is the
> pre-merge gate.

## What it does

- Signs into the FunctionFly orchestrator (email + password, or `ffp_` API key).
- Persists the access + refresh tokens in the OS keychain (Keychain / Credential Manager / libsecret). The webview never sees them.
- Holds the unlock passphrase in process memory only, wiped on lock and on window close.
- Reads, searches, creates, edits, reveals, diffs, rotates, rolls back, and deletes secrets. Decryption **and encryption** happen in the Rust core against PBKDF2-HMAC-SHA256 + AES-256-GCM, with the same wire format as the dashboard (`web/dashboard/src/utils/vault-crypto.ts`).
- Lists namespaces, namespaces, and the audit log.

Secret values are **encrypted locally** before they leave the process: the
webview passes plaintext to the Rust core, which derives the key from the
unlock passphrase and sends only the `encrypted_data` payload to the
orchestrator. The server never sees a plaintext secret.

## Architecture

```
React webview (TypeScript, Vite)
  │
  │  invoke() — typed IPC, single src/lib/ipc.ts wrapper
  ▼
Rust core (Tauri v2 commands)
  │
  ├─ crypto.rs      PBKDF2 + AES-GCM, byte-identical to dashboard
  ├─ api.rs         Orchestrator HTTP client (reqwest + rustls)
  │                  - attaches bearer + X-CSRF-Token
  │                  - retries once on csrf_token_invalid
  ├─ store.rs       OS keychain + in-memory unlock passphrase (Zeroizing)
  └─ error.rs       AppError serialises as {kind, message} over IPC
```

A capability manifest (`src-tauri/capabilities/default.toml`) closes every
default-on Tauri permission except `core:default`, `dialog`, `os`, `process`,
and the explicitly listed `core:window:set-title` + `core:event`. The
webview has no `fs:*`, no `shell:allow-execute`, no `http:*` — every network
call is issued by the Rust core with the bearer attached server-side, so a
compromised webview cannot exfiltrate tokens.

## Build prerequisites

- Rust ≥ 1.80 (`rustup install stable`).
- Node 20+ and `bun` (the dashboard monorepo already pins these).
- Tauri platform dependencies per the [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/):
  - **Linux**: `libwebkit2gtk-4.1-dev`, `build-essential`, `libssl-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`.
  - **macOS**: Xcode Command Line Tools.
  - **Windows**: WebView2 Runtime (preinstalled on Windows 11) + MSVC build tools.

## Development

```bash
cd vault-desktop
bun install
# In one terminal — the Vite dev server for the webview:
bun run dev
# In another — the Rust binary, which loads the Tauri config + the dev URL:
cd src-tauri
cargo run
```

## Production build

```bash
cd vault-desktop
bun install
bun run build                      # tsc --noEmit && vite build
cd src-tauri
cargo tauri build                  # produces platform-specific bundles
```

The output for the host OS lands under `src-tauri/target/release/bundle/`:

| OS      | Format         | Path                     |
| ------- | -------------- | ------------------------ |
| Linux   | AppImage / deb | `bundle/{appimage,deb}/` |
| macOS   | .app / .dmg    | `bundle/{macos,dmg}/`    |
| Windows | .msi / .exe    | `bundle/{msi,nsis}/`     |

Cross-compilation from one host to another is supported but requires the
matching linker; the simplest path is a GitHub Actions matrix on three
runners (see `Deploy / CI`).

## Deploy / CI

> Branching model: [`docs/BRANCHING.md`](docs/BRANCHING.md). `develop` takes
> day-to-day work; `main` only ever holds released, tagged code.

`.github/workflows/`:

| Workflow | Trigger | What it does |
| --- | --- | --- |
| [`ci.yml`](.github/workflows/ci.yml) | push/PR to `main` or `develop` | `tsc` + vitest + vite build, `cargo fmt`/`clippy`/`test`, then a 3-OS compile matrix, plus `bash -n` + self-tests over `scripts/release/`. |
| [`release-please.yml`](.github/workflows/release-please.yml) | push to `develop` | Conventional Commits → SemVer bump → opens the `chore(release): vX.Y.Z` PR into `main`, updating the CHANGELOG and all three version files. |
| [`release.yml`](.github/workflows/release.yml) | `v*` tag | Verifies the tag against every version source, builds + signs installers on 3 OSes, creates a **draft** GitHub Release, and optionally publishes the updater manifest to R2. |
| [`branch-flow.yml`](.github/workflows/branch-flow.yml) | PR to `main`/`develop` | Rejects invalid head→base pairings and non-Conventional-Commit titles. |
| [`codeql.yml`](.github/workflows/codeql.yml) | push/PR + weekly | Static analysis for Rust and TypeScript. |
| [`dependency-review.yml`](.github/workflows/dependency-review.yml) | PR | Vulnerable-dependency diff plus `cargo audit` over `src-tauri/Cargo.lock`. |

Nothing is tagged by hand — merge the release PR and release-please tags it.

### Secrets the release needs

`release.yml` is the only workflow that ever sees the updater signing key.

| Secret | Required | Purpose |
| --- | --- | --- |
| `RELEASE_PLEASE_TOKEN` | for automated releases | PAT with `contents: write` + `pull-requests: write`. The enterprise policy blocks `GITHUB_TOKEN` from creating PRs, so release-please cannot run without this. |
| `TAURI_SIGNING_PRIVATE_KEY` | **yes** | Ed25519 updater signing. Absent → the release fails closed rather than shipping an unsigned artefact the updater would reject anyway. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | **yes** | Passphrase for the above. |
| `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_BUCKET` | no | Publishes the updater manifest. When unset the installers still publish and the job logs how to run `publish-update.sh` by hand. |
| `APPLE_*`, `WINDOWS_CERTIFICATE*` | no | Notarisation / code signing. |

### Known gaps

- **CodeQL does not currently run.** Code scanning needs Code Security, which
  this private repo does not have under the org's enterprise policy. The
  workflow probes for it and skips with a notice rather than sitting red.
- **Automated changelog releases are blocked** for the same class of reason —
  see the manual procedure in [`docs/BRANCHING.md`](docs/BRANCHING.md).

### Local checks that mirror CI

```bash
bun install --frozen-lockfile
bunx tsc --noEmit && bunx vitest run && bunx vite build

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets
cargo test --lib
cargo test --test orchestrator_mock
cargo test --test device_flow
cargo tauri build --no-bundle   # packaging suppressed; see ci.yml for why
```

## Crypto interop guarantee

The Rust core's `crypto::encrypt` / `crypto::decrypt` produce values the
dashboard's `vault-crypto.ts` can decrypt byte-for-byte, and vice versa:

- PBKDF2-HMAC-SHA256, 16-byte salt, 600 000 iterations (`key_version = 2`).
  Legacy `key_version = 1` (100 000 iterations) is read-only.
- AES-256-GCM, 12-byte nonce.
- Ciphertext on the wire is `base64(ct ‖ tag)`, `tag` field empty. The
  `normalized()` method accepts the detached-tag form too so payloads written
  by a Go-native client also decrypt.

Pinned by 13 unit tests in `src-tauri/src/crypto.rs::tests` covering:
roundtrip, wrong passphrase, salt/IV uniqueness, legacy-v1 read, unknown
key-version rejection, detached-tag normalization, AEAD tag tampering,
missing salt, DEK wrap/unwrap, 32-byte DEK length guard, zeroize, and
empty/Unicode/50KB plaintext roundtrip.

## Auth & security

- **Bearer + CSRF**: every mutating call to `/v1/vault/*` requires
  `Authorization: Bearer <token>` **and** `X-CSRF-Token: <csrf>`. The
  Rust core fetches one CSRF token at session start and refreshes it
  once on a 403 with `csrf_token_invalid` in the body. The dashboard
  sidesteps this by using relative URLs; we do not, so the same code
  path works against a remote orchestrator.
- **Captcha**: `POST /v1/auth/login` is captcha-gated server-side when
  `ENVIRONMENT=production`. A 503 from the orchestrator (no captcha
  provider configured) is surfaced as a typed `ApiError::Api { code:
"captcha_not_configured" }` and the UI tells the user to use an
  API key.
- **Refresh tokens are single-use**: the old token is revoked at the
  server before the new one is issued. We persist the new pair
  immediately, so a crash between the swap and the persistence loses
  the session at worst.
- **OS keychain**: `KEYRING_SERVICE = "functionfly-vault-desktop"`
  (distinct from Studio's `functionfly-studio` namespace, so a
  single keychain can host both without collision).

## IPC surface (webview → Rust)

All commands are declared in `src-tauri/src/lib.rs` via
`tauri::generate_handler!`. Every one returns a JSON value or a structured
`AppError { kind, message }`.

| Command                                        | Returns                                                    | Auth required      |
| ---------------------------------------------- | ---------------------------------------------------------- | ------------------ |
| `login`                                        | `{ user, desktop_capable }`                                | —                  |
| `login_with_api_key`                           | same                                                       | —                  |
| `restore_session`                              | `{ user?, refreshed, base_url? }`                          | —                  |
| `logout`                                       | —                                                          | —                  |
| `unlock_vault(passphrase)`                     | —                                                          | session            |
| `lock_vault`                                   | —                                                          | session            |
| `vault_status`                                 | `{ authenticated, unlocked, user? }`                       | —                  |
| `list_secrets(query)`                          | `{ secrets[], total }`                                     | session            |
| `get_secret(id)`                               | `SecretResponse`                                           | session            |
| `create_secret(input)`                         | `SecretResponse`                                           | session + unlocked |
| `update_secret(input)`                         | `SecretResponse`                                           | session            |
| `rotate_secret(input)`                         | `SecretResponse`                                           | session + unlocked |
| `delete_secret(id)`                            | —                                                          | session            |
| `list_secret_versions(id)`                     | `{ versions[], total }`                                    | session            |
| `reveal_secret(id)`                            | `{ secret_id, plaintext }`                                 | session + unlocked |
| `diff_secret_versions(id, from, to)`           | `SecretVersionDiffResponse`                                | session            |
| `rollback_secret(id, target_version, reason?)` | `RollbackSecretResponse`                                   | session            |
| `list_namespaces`                              | `{ namespaces[], total }`                                  | session            |
| `create_namespace(name)`                       | `NamespaceResponse`                                        | session            |
| `delete_namespace(id)`                         | —                                                          | session            |
| `list_tokens(secret_id)`                       | `{ tokens[] }`                                             | session            |
| `generate_token(input)`                        | `TokenResponse` (plaintext once)                           | session + unlocked |
| `revoke_token(token_id)`                       | —                                                          | session            |
| `bulk_delete_secrets(input)`                   | `{ previews, blockers }`                                   | session            |
| `list_audit(secret_id?, limit?)`               | `{ entries[], total }`                                     | session            |
| `list_audit_by_action(action, limit?)`         | `{ entries[], total }`                                     | session            |
| `list_audit_by_actor(actor_type, actor_id, …)` | `{ entries[], total }`                                     | session            |
| `vault_check_update(channel)`                  | `{ result: "up_to_date" }` or `{ result: "available", … }` | — (fail-closed)    |
| `vault_apply_update(channel)`                  | `{ installed: { version } }`                               | — (fail-closed)    |

## Updater (signed auto-update)

The vault ships with a signed auto-update flow backed by
`tauri-plugin-updater`. It mirrors the FunctionFly Studio Phase 5
contract but with the vault-specific env-var names:

| Env var                    | Required | Default                                                                               | Purpose                                                     |
| -------------------------- | -------- | ------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `VAULT_UPDATER_PUBKEY`     | **yes**  | (unset → fail-closed)                                                                 | Ed25519 public key (PEM) printed by `tauri signer generate` |
| `VAULT_UPDATER_STABLE_URL` | no       | `https://updates.functionfly.app/vault/stable/{{target_version}}/{{current_version}}` | Stable-channel update manifest endpoint                     |
| `VAULT_UPDATER_BETA_URL`   | no       | `https://updates.functionfly.app/vault/beta/{{target_version}}/{{current_version}}`   | Beta-channel update manifest endpoint                       |

**Fail-closed contract.** Every `vault_check_update` and
`vault_apply_update` call reads the config from the process env at
call time. A missing or empty `VAULT_UPDATER_PUBKEY` raises a typed
`missing_pubkey` error — the Rust core refuses to fetch any
manifest, the webview surfaces the code in a typed error envelope,
and a blank per-channel URL raises `invalid_endpoint`. There is no
fallback to an unsigned update.

**Operator-side release flow** (the canonical contract is mirrored
in `src-tauri/src/updater.rs`). Three shell scripts ship in
`scripts/release/` to make the build-host side reproducible and
fail-closed:

```bash
# Once per build host — generates <operator key dir>/vault-key{,.json}
cargo-tauri signer generate -w <operator key dir>/vault-key.json
# (64-char password saved to <operator key dir>/vault-key-password.txt)

# Once per environment (staging + production) — provisions the R2 bucket
R2_ACCOUNT_ID=… R2_ACCESS_KEY_ID=… R2_SECRET_ACCESS_KEY=… \
  ./scripts/release/provision-r2-bucket.sh

# Every release — load env, build, publish
source ./scripts/release/load-updater-env.sh
cargo tauri build   # produces both installer + side-car with .sig
./scripts/release/publish-update.sh stable 0.1.1 \
  target/release/bundle/appimage/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz \
  target/release/bundle/appimage/functionfly-vault-desktop_0.1.1_amd64.AppImage.tar.gz.sig
```

| Script                                                   | Idempotent?                     | Touches R2?                              | What it does                                                                                                                                                                                                                                        |
| -------------------------------------------------------- | ------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `load-updater-env.sh`                                    | yes                             | no                                       | Exports `VAULT_UPDATER_PUBKEY`, `TAURI_SIGNING_PRIVATE_KEY_*`; prints a redacted manifest to stderr                                                                                                                                                 |
| `provision-r2-bucket.sh`                                 | yes                             | yes                                      | Creates `functionfly-vault-updates`, enables versioning, applies the noncurrent-version lifecycle rule, sets CORS, runs a write+read+delete round-trip probe                                                                                        |
| `publish-update.sh <channel> <version> <side-car> <sig>` | no (idempotent at the S3 level) | yes                                      | Uploads the side-car + signature + `manifest.json` for the given channel                                                                                                                                                                            |
| `publish-update.test.sh`                                 | yes                             | no (or yes, when `MOCK_R2_STORE` is set) | 7-check arg-parsing smoke + 8th end-to-end round-trip via the bundled `mock-r2.py` (gated on `MOCK_R2_STORE` + `python3` + `aws` being on PATH; otherwise inert). Captured evidence at `scripts/release/evidence/2026-10-06-mock-r2-round-trip.md`. |

Existing installs pick up the new manifest on their next
`vault_check_update(channel)` call (the user clicks **Check for
updates** in the sidebar). The Rust core downloads the
side-car, verifies the Ed25519 signature against
`VAULT_UPDATER_PUBKEY`, and schedules install on next restart.
**Why two channels.** The launch surface ships `stable` and `beta`.
A third "insider" rapid-iteration channel is a deliberate
follow-up — every extra channel multiplies the signing-key
rotation cost and the operator-surface area, and a security-critical
client does not need same-day insider builds.

## Env / config

There are no operator-facing env vars in v0.1.0. The orchestrator URL is
asked of the user at login and persisted in the keychain alongside the
tokens. Per-build settings (window title, identifier, icon) live in
`src-tauri/tauri.conf.json`.

### Brand mark / icons

The full icon set under `src-tauri/icons/` was generated by
`scripts/generate-icons.py` from a 1024×1024 master:

- `icon.png` — 1024×1024 master (Linux / `.deb` / AppImage)
- `icon.ico` — multi-resolution (16/24/32/48/64/128/256) for Windows
- `icon.icns` — macOS multi-resolution Apple icon container
- `32x32.png`, `128x128.png`, `128x128@2x.png` — Linux PNG sizes
- `Square*Logo.png` (10 × 30–310 px) — Windows MSI tile grid
- `StoreLogo.png` — Windows MSI store logo

Re-generate the whole batch:

```bash
python3.12 scripts/generate-icons.py
```

The script is deterministic; rerun it whenever the brand mark changes
(or commit the new icon set in the same PR as any UI re-skin).

## Known limitations

- **No secret-value edit in place.** Metadata (name / description / scopes)
  is edited via `PATCH`; changing the encrypted value is a _rotation_
  (`POST …/rotate`), surfaced as its own tab and recorded in the audit log.
  This matches the orchestrator's API split.
- **No deep-link / OAuth flow** for initial login. The user types
  credentials or pastes an API key; there is no browser-handoff.
- **Captcha in production** blocks email+password login unless the
  orchestrator is configured with a real provider. The UI tells the
  user to use an API key.
- **Encryption/decryption blocks the tokio worker** for ~0.3–1 s per
  operation (600k PBKDF2 iterations). Fine for explicit, low-frequency
  reveal/create/rotate actions; if parallel volume becomes a real
  workload, move the work onto a dedicated blocking thread.

## Manual smoke checklist (gating staging deploy)

Until the headless CI runner has a real OS keychain, the following manual
checks are the merge gate:

1. **Linux**
   - `cd vault-desktop && bun install && bun run build`
   - `cd src-tauri && cargo tauri build` → `target/release/bundle/appimage/*.AppImage`
   - Run the bundle. Confirm the first-launch keychain prompt.
   - Log in against `https://api.functionfly.com`. Confirm the keychain
     now contains `functionfly-vault-desktop` entries via `secret-tool search
service functionfly-vault-desktop` (libsecret).
   - List secrets, reveal one, copy the plaintext into a fresh terminal,
     confirm the value matches what the dashboard reveals for the same
     secret.
   - Create a secret from the desktop (**New secret**), then reveal it and
     confirm the plaintext round-trips. Confirm the dashboard can reveal the
     same secret (proves the wire format is interoperable both ways).
   - **Rotate** the secret with a new value, confirm the version count
     increments, then reveal and confirm the new value. **Roll back** to the
     prior version and confirm the value reverts. **Delete** the secret and
     confirm it disappears from the list.
2. **macOS** — same flow, bundle the `.app`/`dmg`, confirm Keychain entries
   under the `functionfly-vault-desktop` service via
   `security find-generic-password -s functionfly-vault-desktop`.
3. **Windows** — bundle the `.msi`/`.exe`, confirm Credential Manager entries
   under `functionfly-vault-desktop` via `cmdkey /list:functionfly-vault-desktop`.
4. Across all three: lock the vault, confirm the list still loads but
   the **Reveal** button is gated; sign out, confirm `/v1/vault/*` calls now
   return 401 and the UI returns to the login screen.

## Where this code came from

- **Tauri + keyring precedent**: `studio-desktop/src-tauri/` — same Tauri
  version, same `keyring` crate version, same least-privilege capability
  shape. The capability manifest comments reference that file's reasoning
  verbatim.
- **Crypto contract**: `web/dashboard/src/utils/vault-crypto.ts` and
  `sdk/go-vault-sdk/vault/crypto.go` — both authoritative; the Rust port
  is byte-exact.
- **API surface**: `internal/api/handlers/vault/` (handlers),
  `internal/api/handlers/vault/types.go` (request/response shapes),
  `internal/api/middleware/csrf.go` (CSRF contract), and
  `internal/auth/types.go` (login response shape).

## License

See `LICENSE` at the repo root.
