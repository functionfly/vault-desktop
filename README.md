# FunctionFly Vault Desktop

Cross-platform desktop client for the FunctionFly zero-knowledge secrets API.
Built on Tauri v2 (Rust core + React webview) and packed into a single binary
for Windows, macOS, and Linux.

> **Status: 2026-11-03 — v0.1.0.**
> Rust core: `cargo clippy --all-targets -- -D warnings` clean; 52/52 unit tests + 19/19 mockito integration tests + 10/10 device-flow tests + 4/4 updater-config contract tests passing. React bundle: `tsc` + `vite build` clean, 58/58 vitest. Signed auto-update (Phase 5) is wired with the Ed25519 public key compiled into the binary, so an installed client can verify updates without any environment variable.
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
default-on Tauri permission except `core:default`, the explicitly listed
`core:window:set-title` + `core:window:allow-theme` + `core:event`, and
`updater:default`. The webview has no `fs:*`, no `shell:allow-execute`, no
`http:*` — every network call is issued by the Rust core with the bearer
attached server-side, so a compromised webview cannot exfiltrate tokens.

## Build prerequisites

- Rust ≥ 1.90 (`rustup install stable`). The MSRV is 1.90 because the Tauri
  v2 dependency graph declares it — not 1.80, which was declared until
  2026-11-03 and never actually buildable.
- Node 20+ and `bun` (the dashboard monorepo already pins these).
- Tauri platform dependencies per the [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/):
  - **Linux**: `libwebkit2gtk-4.1-dev`, `build-essential`, `libssl-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`.
  - **macOS**: Xcode Command Line Tools.
  - **Windows**: WebView2 Runtime (preinstalled on Windows 11) + MSVC build tools.

## Development

```bash
cd vault-desktop
bun install
# The Tauri CLI is a devDependency — `bunx tauri dev` starts the Vite dev
# server AND the Rust binary, and wires them together. Do NOT run
# `bun run dev` + `cargo run` separately: the standalone `cargo run` does
# not know the dev server's port, and the app comes up pointing at a URL
# nothing is serving.
bunx tauri dev
```

## Production build

```bash
cd vault-desktop
bun install
bun run build                      # tsc --noEmit && vite build
bunx tauri build                   # produces platform-specific bundles
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
| `RELEASE_PLEASE_TOKEN` | for automated releases | PAT with `contents: write` + `pull-requests: write`. The CI policy blocks `GITHUB_TOKEN` from creating PRs, so release-please cannot run without this. |
| `TAURI_SIGNING_PRIVATE_KEY` | **yes** | Ed25519 updater signing. Absent → the release fails closed rather than shipping an unsigned artefact the updater would reject anyway. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | **yes** | Passphrase for the above. |
| `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_BUCKET` | no | Publishes the updater manifest. When unset the installers still publish and the job logs how to run `publish-update.sh` by hand. |
| `APPLE_*`, `WINDOWS_CERTIFICATE*` | no | Notarisation / code signing. |

### Known gaps

- **Static analysis does not currently gate merges.** Code scanning requires a
  Code Security licence this repository's org does not hold. The workflow
  probes for it and skips with a notice rather than sitting red, so it
  re-enables itself the moment the licence is added.
- **Automated changelog releases are blocked** for the same class of reason —
  see the manual procedure in [`docs/BRANCHING.md`](docs/BRANCHING.md).

> Deployment specifics (bucket identifiers, key paths, owner handles, and the
> org's CI policy constraints) are deliberately kept out of this document in
> [`docs/INTERNAL.md`](docs/INTERNAL.md), which is not published.

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
cargo test --test updater_config
cargo tauri build --no-bundle   # packaging suppressed; see ci.yml for why
```

Or run the whole gate set in one command, exactly as CI runs it:

```bash
scripts/ci.sh                # every gate
scripts/ci.sh --quick        # skip the slow Tauri compile
scripts/ci.sh --job rust     # one job only
scripts/ci.sh --report       # also publish a check run to the pull request
```

### Running CI outside GitHub Actions

`scripts/ci.sh` runs the same gates as `.github/workflows/ci.yml` on any
machine — a laptop, a VPS, WSL, or the container in
`scripts/ci/Dockerfile`. It needs no GitHub account, no runner
registration and no Actions minutes:

```bash
# Docker (reproducible, needs only a Docker daemon)
docker build -f scripts/ci/Dockerfile -t vault-ci .
docker run --rm -v "$PWD:/src" -w /src vault-ci

# Or directly, on any machine with bun + cargo + python3
scripts/ci.sh
```

Nothing in the gate set needs the GitHub Actions *runtime* — every step is
a shell command. The only GitHub dependency is the REST API, and that
works fine; `--report` publishes the outcome back to the pull request as
a check run.

The same applies to the release pipeline. `release-please` is a CLI
(`npx release-please release-pr`), not an Action, and the signed builds
are `tauri build` — so a release can be cut from any machine that holds
the signing key:

```bash
npx release-please release-pr --repo-url=functionfly/vault-desktop \
  --target-branch=main --config-file=release-please-config.json \
  --manifest-file=.release-please-manifest.json
```

> **Current status (2026-11-03):** GitHub Actions is failing in the
> `functionfly` organisation with `startup_failure` — the run is created
> but no job ever starts, and there is no log. It affects every workflow
> in every repository in the org, including a push of an unmodified
> commit whose CI succeeded the week before, so it is not caused by
> anything in this repository. `scripts/ci.sh` is the workaround until an
> org admin restores it.

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

## Tests

| Suite                                        | Count | Command                                  |
| -------------------------------------------- | ----- | ---------------------------------------- |
| Rust unit tests                              | 52    | `cargo test --lib`                       |
| Orchestrator mock (HTTP + CSRF contract)    | 19    | `cargo test --test orchestrator_mock`    |
| Device-flow (RFC 8628 device grant)         | 10    | `cargo test --test device_flow`          |
| Updater config contract                      | 4     | `cargo test --test updater_config`       |
| Frontend IPC + theme contract                | 58    | `bunx vitest run`                        |
| **Total**                                    | **143** | |

The release tooling has its own suite — `bash scripts/release/publish-update.test.sh` —
which asserts the updater manifest contract rather than merely checking that
a manifest file exists.

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
| `begin_device_login`                           | `DeviceCodePrompt`                                         | —                  |
| `await_device_login(input)`                    | `{ user, desktop_capable }`                                | —                  |
| `restore_session`                              | `{ user?, refreshed, base_url? }`                          | —                  |
| `logout`                                       | —                                                          | —                  |
| `unlock_vault(passphrase)`                     | —                                                          | session            |
| `lock_vault`                                   | —                                                          | session            |
| `vault_status`                                 | `{ authenticated, unlocked, user? }`                       | —                  |
| `vault_health_probe`                           | `{ reachable, latency_ms? }`                               | —                  |
| `vault_store_info`                             | `{ backend, location? }`                                   | —                  |
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

| Env var                    | Required | Default                                                                    | Purpose                                                      |
| -------------------------- | -------- | -------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `VAULT_UPDATER_PUBKEY`     | no       | the key compiled into the binary (see below)                               | Ed25519 public key **override** — point a build at a rotated or staging key |
| `VAULT_UPDATER_STABLE_URL` | no       | `https://updates.functionfly.app/vault/stable/manifest.json`              | Stable-channel update manifest endpoint                      |
| `VAULT_UPDATER_BETA_URL`   | no       | `https://updates.functionfly.app/vault/beta/manifest.json`                | Beta-channel update manifest endpoint                        |

> **The endpoint is the channel's static manifest.** It carries no version
> placeholder. `tauri-plugin-updater` substitutes only `{{current_version}}`,
> `{{target}}`, `{{arch}}` and `{{bundle_type}}` — there is no
> `{{target_version}}` token, and a URL containing one is requested
> verbatim and 404s.

**Where the verification key comes from.** The Ed25519 public key lives in
`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`, and Tauri compiles
it into the binary. That is what makes auto-update work for an ordinary
user, who installed the app and has no environment variables set.

`VAULT_UPDATER_PUBKEY` is an **override**, not the primary source: when it
is set to a non-blank value it wins, so an operator can point a build at a
rotated or staging key without rebuilding. The resolution order is:

1. `VAULT_UPDATER_PUBKEY` (if set and non-blank)
2. `plugins.updater.pubkey` compiled into the binary
3. neither → typed `missing_pubkey` error, no manifest is fetched

**Fail-closed contract.** Every `vault_check_update` and
`vault_apply_update` call resolves the key at call time. If no key is
available at all, the call raises a typed `missing_pubkey` error — the
Rust core refuses to fetch any manifest, the webview surfaces the code in
a typed error envelope, and a blank per-channel URL raises
`invalid_endpoint`. There is no fallback to an unsigned update.

> **Do not delete the `plugins.updater` block from `tauri.conf.json`.**
> It looks like dead config because `commands/updater.rs` overrides the
> endpoints at runtime — but the plugin's `Config` struct has a
> `pubkey: String` field with no serde default, so an absent section
> deserialises as `null` and the app **panics on startup**:
>
> ```
> PluginInitialization("updater", "Error deserializing 'plugins.updater' within
> your Tauri configuration: invalid type: null, expected struct Config")
> ```
>
> The empty `endpoints` list is an intentional placeholder: the per-channel
> URLs are supplied at runtime. The `pubkey` is **not** a placeholder — it
> must hold the real key, because the Tauri CLI reads this same field when
> it signs the updater artefacts. An empty pubkey makes the *release build*
> fail at the signing step, which no CI gate would otherwise catch (CI
> builds with `--no-bundle`, which produces nothing to sign).

**Keeping the two keys in sync.** The CLI signs artefacts with
`TAURI_SIGNING_PRIVATE_KEY` but reads the verification key from
`tauri.conf.json`. If the two ever drift apart, the release still builds
and still publishes — and every update fails verification on a user's
machine. `scripts/release/verify-updater-keypair.sh` turns that into a
hard failure and runs in CI and in `release.yml`.

**Operator-side release flow** (the canonical contract is mirrored
in `src-tauri/src/updater.rs`). Three shell scripts ship in
`scripts/release/` to make the build-host side reproducible and
fail-closed:

```bash
# Once per build host — generates the signing keypair under $VAULT_KEY_DIR
cargo-tauri signer generate -w "$VAULT_KEY_DIR/vault-key.json"

# Once per environment (staging + production) — provisions the R2 bucket
R2_ACCOUNT_ID=… R2_ACCESS_KEY_ID=… R2_SECRET_ACCESS_KEY=… \
  ./scripts/release/provision-r2-bucket.sh

# Every release — load env, build, publish
source ./scripts/release/load-updater-env.sh
bunx tauri build   # produces installer + side-car + .sig
# Publish every artefact in a directory, one manifest for all platforms:
./scripts/release/publish-updater-assets.sh stable 0.1.1 ./sidecars
```

The manifest is the contract `tauri-plugin-updater` reads. Three properties
are load-bearing, and each one has broken a release in the past:

| Property          | Value                                                                  | Why                                                                                       |
| ----------------- | ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `platforms` keys  | `{os}-{arch}` and `{os}-{arch}-{installer}`                            | The plugin looks up exactly these. A Rust target triple (`x86_64-unknown-linux-gnu`) is never found. |
| `signature`       | The **content** of the `.sig` file (base64 minisign signature)          | The plugin base64-decodes this field and minisign-verifies the download against it. A URL here fails verification at install time. |
| `notes`           | A JSON string                                                          | The manifest is parsed as JSON. Shell-quoting the notes produces a document no client can parse. |

The installer format is part of the key because a `.deb` user must be offered
the `.deb` archive, not the AppImage that shares its architecture.
`scripts/release/updater-targets.py` owns that mapping and is asserted by the
release test suite.

| Script                                                    | Idempotent?                      | Touches R2?                              | What it does                                                                                                                                                                                                                     |
| --------------------------------------------------------- | -------------------------------- | ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `load-updater-env.sh`                                     | yes                              | no                                       | Exports `VAULT_UPDATER_PUBKEY`, `TAURI_SIGNING_PRIVATE_KEY_*`; prints a redacted manifest to stderr                                                                                                                              |
| `provision-r2-bucket.sh`                                  | yes                              | yes                                      | Creates the updater bucket, enables versioning, applies the noncurrent-version lifecycle rule, sets CORS, runs a write+read+delete round-trip probe                                                                                  |
| `publish-update.sh <channel> <version> <side-car> <sig> [target]` | no (idempotent at the S3 level) | yes                                      | Uploads the side-car + signature and merges that platform's entry into `vault/<channel>/manifest.json`. Accepts a comma-separated target list for a multi-arch archive. |
| `publish-updater-assets.sh <channel> <version> <dir>`    | no                               | yes                                      | Publishes every updater artefact in a directory, building one manifest across all platforms.                                                                                                                                    |
| `render-manifest.py`                                      | yes                              | no                                       | The manifest contract, in one place. Emits valid JSON; refuses a target triple, a non-base64 signature, or a repointed platform.                                                                                                    |
| `updater-targets.py`                                      | yes                              | no                                       | Maps an artefact filename to the manifest target keys the plugin looks up.                                                                                                                                                        |
| `publish-update.test.sh`                                  | yes                              | no (or yes, when `MOCK_R2_STORE` is set) | Arg-parsing smoke + the manifest contract assertions + the artefact→target mapping. The mock-R2 round-trip needs `python3` + `aws` and is otherwise inert.                                                |

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

The orchestrator URL is asked of the user at login and persisted in the
keychain alongside the tokens. Per-build settings (window title, identifier,
icon) live in `src-tauri/tauri.conf.json`.

The updater reads its configuration from the process environment at call
time — see [Updater (signed auto-update)](#updater-signed-auto-update) for
the full list. `VAULT_ORCHESTRATOR_URL` optionally seeds the login screen's
default orchestrator URL; it is a convenience, not a requirement.

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
