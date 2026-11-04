<!--
  The title is validated by .github/workflows/branch-flow.yml and drives the
  SemVer bump in the CHANGELOG. Getting the type right is what makes the
  release number correct.
    fix / perf / refactor / revert  -> patch
    feat                             -> minor
    BREAKING CHANGE or a trailing ! -> major
-->

## What this changes

<!-- One or two sentences. Link the issue if there is one. -->

## Why

<!--
  For anything touching src-tauri/src/crypto.rs, store.rs, api.rs, or
  capabilities/: state the security impact explicitly.
-->

## How it was verified

- [ ] `bun run build` (tsc + vite)
- [ ] `bunx vitest run`
- [ ] `cargo test --lib` (in `src-tauri`)
- [ ] `cargo test --test orchestrator_mock`
- [ ] Cross-platform: compile-checked on Linux, macOS and Windows (CI matrix)

## Security impact

<!--
  Required for changes under src-tauri/. Answer all three:
    - Does this change what the webview is permitted to invoke?
    - Does this change the ciphertext/key derivation on the wire?
    - Does this change how the bearer token or CSRF token is handled?
  If any answer is "yes", the crypto-interop note must say whether the
  dashboard or the Go SDK needs a matching change.
-->

- Webview capability surface changed: <!-- yes / no -->
- Crypto wire format changed: <!-- yes / no -->
- Token handling changed: <!-- yes / no -->

<!-- If the crypto format changed, note whether this breaks round-tripping
     against `web/dashboard/src/utils/vault-crypto.ts` or
     `sdk/go-vault-sdk/vault/crypto.go`. -->

## Manual verification

<!--
  CI has no OS keychain, so anything that touches real keychain or updater
  behaviour cannot be proven in CI. List what a human must check, or say
  "none".
-->

## Risk / rollback

<!--
  How do we undo this if it is wrong in production? For an updater-affecting
  change, say whether the release can be pulled from the R2 channel.
-->