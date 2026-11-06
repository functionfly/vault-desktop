# Changelog

All notable changes to FunctionFly Vault Desktop are recorded here. The
project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.2] — 2026-11-05

A dependency-security release. The v1.0.1 installers were built before the
`rustls` upgrade and therefore still carry it, so this release exists to
put the fixed dependency in front of users. There is no behaviour change:
no feature, no fix to the application's own logic, no API change.

### Security

- **`rustls` was upgraded from 0.23.45 to 0.23.46**, clearing four open
  advisories that GitHub's dependency scanner had flagged against the
  shipped binaries:

  | Advisory | Severity | Issue |
  | --- | --- | --- |
  | `GHSA-vp88-798v-h67f` | low | Buffered application data was sent to the server even when the server rejected an Encrypted Client Hello, before the `ech_required` alert |
  | `GHSA-w44r-93j7-qv28` | low | An unauthenticated `ServerHello` could poison the per-name key-exchange record |
  | `GHSA-c8m5-c952-6cm4` | medium | `Debug` formatting of a client session value exposed the TLS 1.2 master secret or the TLS 1.3 resumption PSK |
  | `GHSA-h2v8-394m-p7vv` | low | The (disabled-by-default) Brotli certificate decoder could be made to allocate up to 1 GiB before authentication |

  0.23.46 is the release that carries all four fixes. The bump is a
  lockfile-only change to a transitive dependency — `rustls` is not a
  direct dependency, it arrives through `reqwest`/`hyper-rustls` — so no
  source file changed and no API surface moved. The upstream release
  also credits fixes submitted by OpenAI's outbound-disclosure
  programme.

  Two of the four were reachable in principle: the master-secret leak
  only through a custom session store that logs `Debug` output (the
  app uses the built-in cache, and the built-in diagnostics stay
  redacted), and the ECH ordering bug only when ECH is configured and
  the server rejects it. The Brotli decoder is off by default. The
  key-exchange-record poisoning needs an active network attacker. None
  of them are exploitable against the shipped build, but shipping a
  dependency with a known, fixed advisory is a different posture from
  shipping one with an open one, so the upgrade was made rather than
  the alerts dismissed.

  The fifth alert, `GHSA-wrw7-89jp-8q8g` (medium, unsoundness in
  `glib::VariantStrIter`), is **accepted rather than fixed**: the fix
  was merged into the `gtk-rs-core` tree after the 0.18.x line was
  frozen, so there is no in-range upgrade to take. `glib` reaches this
  project only through Tauri's GTK stack, which Tauri gates behind
  `cfg(any(target_os = "linux", …))` — so it is absent from the Windows
  and macOS dependency graphs, but it **is** in the Linux graph and the
  shipped AppImage and `.deb` contain it. The vault core calls none of
  the affected API, and the GTK stack is not a path this application
  exercises. The reasoning is written up in
  [`docs/SECURITY-REVIEW.md`](docs/SECURITY-REVIEW.md#dependency-advisories).

### Fixed

- **The release self-tests could fail on a busy build host.** Both
  `scripts/staging-smoke.test.sh` and
  `scripts/release/publish-update.test.sh` start a Python mock and then
  wait for it to come up — by polling its log for the `LISTENING` line,
  and by polling a health check respectively. The budgets were 10
  seconds and 2 seconds.

  That is not enough when the host is busy. The mock has to be scheduled
  before it can start importing, so a machine running a Rust release
  build with LTO in parallel can leave it waiting well past two seconds.
  The failure was also a misleading one: `staging-smoke.test.sh`
  reported *"the mock never reported a port"* with an **empty** log,
  which reads like a broken mock rather than a slow start. It failed the
  portable CI run on 2026-11-05 that way, while the same script passed
  when run on its own.

  Both budgets are now 30 seconds. The waits stay bounded, and the
  "the mock died on startup" branch still separates a crash from a slow
  start.

### Added

- **A macOS compile gate.** `scripts/macos-typecheck.sh` type-checks the
  `aarch64-apple-darwin` build on a Linux host, so a macOS-only compile
  error is caught without waiting for a macOS runner. It cross-compiles
  with `zig cc` (which bundles the macOS libc headers and emits Mach-O
  objects) and LLVM's `llvm-install-name-tool` (which CMake requires for
  a Darwin build). It is a **compile gate, not a build**: it produces no
  `.app` and no `.dmg`, because that needs the Apple SDK, the Tauri macOS
  bundler and an Apple Developer ID certificate. It runs as a job in the
  portable CI runner and in `ci.yml`, and it skips itself with an
  explicit notice on a host without zig — a silent skip that reads as
  "ok" is the worst possible outcome.

  The gate was verified to actually catch errors: a deliberate compile
  error inside a `cfg(target_os = "macos")` block fails the gate, and
  the gate passes again once the error is reverted.

### Changed

- **The GitHub Actions minutes are now spent where they earn their
  cost.** A macOS runner costs 10× a Linux one and a Windows runner 2×,
  so the 3-OS compile matrix now runs **only on a push to `main`** — the
  branch that holds released, tagged code — and not on every pull
  request. macOS compile errors are still caught on every push, because
  the macOS gate type-checks the macOS build on a *Linux* runner.
  CodeQL no longer analyses the same commits twice (a push to `develop`
  and the pull request that carries it), a push or pull request that
  only touches `**/*.md` or `docs/**` runs nothing at all, and the zig
  toolchain is cached instead of being re-downloaded (55 MB) on every
  run. The rules are documented in the README.

- The README now states plainly that there is **no macOS installer**,
  why (the release workflow's `macos-latest` job cannot start while the
  organisation's GitHub billing is unsettled), and that the Ed25519
  updater signature is not a platform vendor signature.


## [1.0.1] — 2026-11-04

### Fixed

- **The app rendered a blank window.** `package.json` declared `react` as
  `^19.3.0` and `react-dom` as `^19.3.1`. Those resolved to react@19.3.0
  and react-dom@19.3.1, and react-dom@19.3.1 declares
  `peerDependencies: { react: "^19.3.1" }` — so the install violated its
  own peer requirement, and React 19 threw:

  > Incompatible React versions: The "react" and "react-dom" packages
  > must have the exact same version.

  The throw happens inside react-dom's client entry point, before the
  app mounts, so there was no UI and no visible error. Both packages
  are now pinned to the exact same version, and a test asserts they
  stay that way — including that the declared `react` satisfies
  react-dom's own peer requirement.

- **Republishing a version does not work.** The CDN caches
  `vault/<channel>/<version>/…`, and the cache outlives the object:
  overwriting the object in R2 does not change what the edge serves. So
  a republished version can have a NEW signature in the manifest while
  the CDN still serves the OLD bytes, and the update is rejected at
  install time with an error that looks like a signing fault rather
  than a cache fault. A new version is a new path, and so a new cache
  entry — which is why this release exists. See the note in
  `scripts/release/publish-update.sh`.

## [1.0.0] — 2026-11-04 — public launch

The first public release of FunctionFly Vault Desktop, and the first
release that is downloadable, updatable and backed by a signed artefact.

### Security

These are the reasons this release exists. `docs/SECURITY-REVIEW.md` is
the full STRIDE review of the IPC surface.

- **The export/import commands took a path from the webview.** That made
  the Rust core a confused deputy. The webview has no `fs:*` scope, but
  it could name any path and have the core read or write it — so the
  missing scope bought nothing. A renderer compromised by an XSS could
  read any file the process could read, and overwrite any file it could
  write. The path parameter is gone: the core opens the native file
  dialog and the user picks the location, so the webview cannot choose
  where the data goes. `dialog:*` is deliberately NOT granted to the
  webview — the dialog belongs to the core, and granting it would let
  the renderer learn the chosen path.
- **The webview had no Content Security Policy.** `tauri.conf.json`
  shipped `"csp": null`, which disables CSP entirely. It now has a
  restrictive one: `script-src 'self'` with no inline script and no
  `eval`, `connect-src` limited to the IPC origins, and
  `object-src` / `base-uri` / `form-action` / `frame-ancestors` at
  `'none'`.
- **Plaintext was one log line away.** `crypto::DecryptedSecret` derived
  `Debug` and held a plaintext field. It was dead code, so it was
  deleted. `EncryptedData` now has a hand-written `Debug` that reports
  only the shape of the envelope — the ciphertext, nonce and salt are
  the entire offline attack surface, and they are no longer printable.
- **The leak guard was failing and nobody saw it.** The R2 account id
  had been pasted into `scripts/release/README.md` — a publishable file
  — since the CDN fix. The guard DID catch it, but its failure message
  goes to stderr and was being swallowed by the local runner, so the
  run looked like a pass. The guard now scans the WHOLE repository
  rather than a hand-maintained list of "publishable" files (a file
  nobody thought to add to that list was silently exempt), and it
  reports its failures on stdout as well as stderr. It was verified to
  both catch a planted leak and pass clean.
- **The build host's home directory was in the source tree.** The
  artefact-contract test hardcoded the operator's key path. It now reads
  the path from `VAULT_SIGNING_PUBKEY`.
- `src-tauri/tests/security_contract.rs` pins the whole posture: the
  export/import commands take no path, the capability manifest grants no
  `dialog:` / `fs:` / `shell:` / `http:`, a CSP is configured, and no
  plaintext-bearing type derives `Debug`. Each guard was verified to
  fail when the property it protects is broken.

### Fixed

- **The secret search box never filtered anything.** The orchestrator's
  `GET /v1/vault/secrets` handler parses `limit`, `offset`,
  `secret_type`, `namespace` and `name` — and ignores `search`
  entirely. The desktop client sent `search=...` on every keystroke
  and got the tenant's whole secret list back each time, so the field
  looked broken. The filter now runs in the Rust core, and the core
  pages through the whole vault (the server caps a page at 100) so a
  match beyond the first page is still found. The match covers the
  name, description, namespace, type and scopes — never the
  ciphertext.
- **The secret-type filter never applied.** `ListQuery.secret_type`
  was declared in the IPC layer and never sent. It is now wired
  through to the orchestrator's `secret_type` query parameter.
- **A platform-held secret reported "wrong passphrase".** A secret with
  `zk_mode: false` is encrypted by the platform with the tenant's
  data-encryption key, so there is no client-side envelope to open and
  the unlock passphrase cannot derive the right key. Attempting the
  decrypt produced a misleading `CryptoError::DecryptFailed`. The
  client now refuses explicitly with a typed `platform_held` error,
  and the UI says so in plain language instead of offering a
  "Reveal" button that could only fail.
- **The per-actor audit filter was unreachable.** `list_audit_by_actor`
  existed in the IPC layer from the start; the audit screen only ever
  used the per-action route and its own comment called the rest "a
  follow-up". Both the audit screen and a new per-secret audit drawer
  now expose it.
- **The updater channel was unreachable.** `updates.functionfly.com`
  resolved but nothing answered it: the R2 bucket had no custom domain
  attached, so the only way to reach it was the S3 API with
  credentials — and an installed client has none. The manifest also
  pointed at a host the bucket was never attached to, and the artefact
  filename embedded a space, which is not fetchable as a URL. All three
  are fixed, and provisioning now fails if the public path does not
  answer an anonymous GET.

### Added

- **Argon2id (`key_version: 3`).** The client could only read
  `key_version` 1 and 2, so an Argon2id envelope — what the
  orchestrator's client-wrap path stamps
  (`internal/storage/vault/client_crypto.go`) — was rejected outright.
  The parameters match `DefaultArgon2Params()` in the Go SDK
  (`sdk/go-vault-sdk/vault/kdf.go`): 64 MiB, 3 iterations, 4 lanes.
  Versions 1 and 2 keep their exact PBKDF2 meaning, so the dashboard
  and the other SDKs are unaffected.
- **DEK envelopes.** `encrypt_under_dek` / `decrypt_under_dek` — the
  byte-exact counterpart of `EncryptUnderDEK` in the Go SDK — so the
  desktop client can read a value sealed under a tenant's
  data-encryption key. `generate_dek` mints fresh key material.
- **Platform-held secrets.** The create dialog can now ask the platform
  to hold the value (`server_side: true`) instead of encrypting it
  locally. This widens the trust boundary — the platform can read the
  secret — so it is opt-in, explained in the UI, and gated on the
  ADMIN tenant role by the orchestrator. It is also the ONLY kind of
  secret a capability "use" grant can serve.
- **Audit chain integrity.** A new "Chain integrity" panel reports the
  tenant's audit hash chain: its posture, its segments, whether each
  anchor still verifies, and the first entry that fails verification.
  The panel is careful to distinguish a KEYED chain (tamper-evident)
  from an unkeyed one (detects accidental corruption only) rather than
  implying the log is proof in either case.
- **Capability grants.** List, approve, deny and revoke scoped access
  requests, and report whether a secret is platform-held.
- **Dynamic credentials.** Manage database targets, create credential
  definitions, mint a live credential and its lease, renew or revoke a
  lease, and revoke every lease at once. A minted credential's
  username and password are shown exactly once and never persisted.
- **Tenant vault MFA.** Read the tenant's vault MFA policy and verify
  a TOTP code. The code travels in the `X-MFA-Code` header — never in
  a query string, because proxies log query strings. Previously a
  tenant with vault MFA enforced could not use the desktop client at
  all.
- **Sharing and federation.** Share a platform-held secret with
  another tenant member, list what has been shared with you, revoke a
  share, and list and create federation keys.
- **Idle auto-lock.** The vault now locks after an idle period
  (default 5 minutes; 0 disables it). The countdown lives in the Rust
  core, not the webview: if the timer lived in JavaScript, a
  compromised renderer could simply not run it. The webview only
  reports activity.
- **Clipboard auto-clear.** A revealed secret copied to the clipboard
  is wiped after 30 seconds. The wipe compares the clipboard against
  the copied value first, so it never destroys something the user
  copied afterwards. If the clipboard cannot be read, the wipe happens
  anyway — failing to read is not evidence that it holds something
  else.
- **Import / export.** Export the tenant's secret METADATA to a JSON
  file and read it back. The export carries no ciphertext and no
  plaintext. The file is written by the Rust core, so the webview
  still has no filesystem scope.
- **Per-secret audit drawer.** A secret's own audit trail, with an
  actor-type and actor-id filter.
- **A platform-held badge** in the secret list and in the detail view.
- **A per-channel version index.** `vault/<channel>/versions.json`
  accumulates every published version with its notes and per-platform
  files, so a downloads page can list the history rather than only the
  newest release. The updater manifest still describes exactly one
  version — that is what the plugin needs — but a client on a platform
  that was not rebuilt must not be offered a stale archive, so the two
  documents differ deliberately.

### Known limitations

- **The installers are not code-signed.** macOS Gatekeeper and Windows
  SmartScreen will warn on first launch. The configuration is in place
  (`entitlements.plist`, hardened runtime, SHA-256 digesting, a
  timestamping server) and signing happens the moment the certificates
  are provisioned; the certificates themselves need a paid Apple
  Developer Program membership and a Windows code-signing certificate.
- **GitHub Actions is down organisation-wide**, so the release pipeline
  was executed by hand from the build host rather than in CI. The
  portable runner (`scripts/ci.sh`) runs the same gates locally.

### Tests

- **125 Rust tests** — 80 unit (`cargo test --lib`: AES-256-GCM
  round-trips, Argon2id envelopes, DEK wrap, detached-tag normalisation,
  unknown-`key_version` rejection), 19 mock-orchestrator integration
  (login round-trip, CSRF fetch + retry, non-loopback base-URL
  rejection, fail-closed without a token, captcha-required surfacing),
  10 device-flow (the RFC 8628 device-grant exchange), 6 security
  contract, 4 updater-config, 4 updater-artefact contract, 2
  OS-keychain.
- **100 vitest contract tests** — every typed wrapper in
  `src/lib/ipc.ts` sends the right `(command, args)` shape; typed
  `{kind, message}` rejections parse into `VaultIpcError`; the theme
  and clipboard helpers behave under hostile storage.
- **Total: 225 tests, all passing.** `cargo clippy --all-targets -- -D
  warnings` is clean, and `tsc --noEmit` is clean.

### What shipped

- **Windows** — an MSI and an NSIS installer, both signed with the
  Ed25519 updater key. Published to the `stable` updater channel at
  `https://updates.functionfly.com/vault/stable/manifest.json`, and
  verified over the wire: the published artefact downloads and its
  signature validates against the key compiled into the binary.
- **Linux** — an AppImage and a deb, signed the same way, built in the
  portable CI container (which carries the Tauri v2 Linux build
  dependencies).
- The repository is **public**, with branch protection on `main` and
  `develop`. The gates are the portable runner (`scripts/ci.sh`), which
  publishes its result back to the pull request as a check run —
  GitHub Actions parses workflows in this organisation but cannot get a
  runner assigned, so it never gates anything.
- The interface is documented in [`docs/screenshots/`](docs/screenshots),
  rendered in a browser against the app's own React code.

### Known gaps in this release

- **macOS is not part of this release.** The macOS artefact requires a
  macOS build host, and GitHub Actions cannot get a runner assigned in
  this organisation. The release ships Windows and Linux; macOS follows
  when a build host is available.
- **The installers are not code-signed by a platform vendor.** The
  updater signature protects the update channel; it does not satisfy
  Gatekeeper or SmartScreen.
- **The CDN cache outlives the object.** A republished version is
  served stale bytes until the cache expires; a new version is needed
  for a corrected release. Purging the cache needs a Cloudflare token
  with cache-purge permission, which the operator's current tokens do
  not have.

## [0.1.0] — 2026-11-03 — preview build

> **Corrected 2026-11-04.** This release was previously described here
> as the "first public release". That was not accurate: it was published
> to the **`beta` updater channel only**, the repository was private, and
> no git tag or GitHub Release was ever created. It was a preview build
> for the beta channel, and nothing else.

### Highlights

- **FunctionFly Vault, as a Linux preview build.** A standalone
  desktop client built with Tauri v2 + React 19. Login with email+password
  or an API key, unlock the vault with a passphrase, browse + reveal +
  rotate + rollback secrets. Only the **Linux x86_64 AppImage** was
  ever built for this release — the macOS and Windows binaries did not
  exist yet.
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
- 2 OS-keychain tests (`cargo test --test keychain_smoke`) — a session
  round-trips through the real credential store. Fails hard if the
  encrypted file fallback is used on Windows or macOS, where a keychain
  always exists; skips with a notice on a headless Linux box.
- 58 vitest contract tests (`bunx vitest run`) — every typed wrapper
  in `src/lib/ipc.ts` sends the right (command, args) shape; typed
  `{kind, message}` rejections parse into `VaultIpcError`.
- **Total: 145/145 tests pass** (52 Rust lib + 19 mockito integration +
  10 device flow + 4 updater config + 2 keychain + 58 vitest).

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
  `https://updates.functionfly.com/vault/{channel}/manifest.json`
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
  and Windows `.msi` / `.exe`. The cross-platform compile matrix runs in
  `.github/workflows/ci.yml`.

### Manual gates

- `scripts/staging-smoke.sh` — 14-step real-orchestrator end-to-end
  script. Idempotent (per-run `STAGING_SMOKE_NONCE`); cleans up after
  itself.
- `scripts/staging-smoke.test.sh` — drives the **unmodified** script
  against a mock orchestrator that enforces the bearer + CSRF contract,
  and asserts the script never triggered a rejection and never sent a
  CSRF header on a safe method. The script had never been run against
  anything before this, so nothing about it was known to work.
- `scripts/keychain-smoke.sh` — round-trips a session through the real
  OS credential store using the application's own `SecretStore` code,
  confirms it through the platform's own tooling, and removes it.
  Replaces a manual per-OS checklist step that had never been executed.
- `scripts/release/check-signing-readiness.sh` — reports whether the
  host could produce a signed, notarised installer. Advisory by
  default; `--require` fails the release. Runs in `release.yml` before
  the build.

### Code signing

- `bundle.macOS.entitlements` (`src-tauri/entitlements.plist`) with the
  hardened runtime enabled, and `bundle.windows` with SHA-256 digesting
  and a timestamping server. The configuration is in place so signing
  works as soon as the certificates are provisioned; the certificates
  themselves need a paid Apple Developer Program membership and a
  Windows code-signing certificate.
- `signingIdentity` and `certificateThumbprint` are deliberately NOT
  set in `tauri.conf.json`: a value that names a certificate which is
  not present makes the build fail outright. Both are supplied at build
  time through `APPLE_SIGNING_IDENTITY` and `WINDOWS_CERTIFICATE`.

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
