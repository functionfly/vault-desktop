# STRIDE review — FunctionFly Vault Desktop v1.0.0

**Reviewed:** 2026-11-04
**Scope:** the Tauri IPC surface (webview → Rust core), the capability
manifest, the crypto module, the store, and the updater contract.
**Reviewer:** engineering (pre-launch pass, release blocker for 1.0.0)

The capability manifest carried this instruction before this review:

> SECURITY REVIEW PENDING (security-specialist): do NOT widen this manifest
> without a STRIDE review of the IPC surface.

This is that review. It is a desk review of the code as it stood at the
1.0.0 launch candidate, not a penetration test.

---

## The trust model

Three boundaries, in order of decreasing trust:

```
┌──────────────────────────────────────────────────────────┐
│ ORCHESTRATOR (remote, semi-trusted)                      │
│   holds bearer/CSRF tokens, tenant metadata, ciphertexts  │
└──────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────┐
│ RUST CORE (trusted)                                       │
│   keychain, passphrase, HTTP client, file IO, crypto     │
└──────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────┐
│ WEBVIEW (untrusted — assume XSS)                         │
│   React UI; holds plaintext only transiently, on reveal  │
└──────────────────────────────────────────────────────────┘
```

**The orchestrator is not trusted with plaintext.** Zero-knowledge is the
product promise: `zk_mode: true` secrets are encrypted in the core before
they leave the process. The orchestrator can see ciphertext, metadata and
audit trails. It cannot see a secret's value.

**The webview is not trusted with anything persistent.** It is the most
likely place for an XSS to land, so the design assumes a compromised
renderer and asks what it can reach.

The capability manifest is the enforcement point for that last row.
`src-tauri/capabilities/default.toml` closes every default-on Tauri
permission except four explicitly listed ones plus `updater:default`.

---

## Findings

### F1 — Arbitrary file read/write through the export/import commands
**Severity: HIGH · Status: FIXED (v1.0.0) · Category: Tampering / Information disclosure**

`export_secrets(path)` and `read_secret_export(path)` accepted the path
as an IPC argument.

The webview has no `fs:*` scope, which was the stated mitigation — but
the core performed the IO on the webview's behalf. The mitigation was
theatre: the missing scope bought nothing, because the webview did not
need a scope to name a path.

Consequences of the original shape:

* **Arbitrary read.** A renderer compromised by XSS could name any path
  the process can read. The command parsed the result as a vault export
  and returned a JSON summary, and the serde error message included
  fragments of the file's contents.
* **Arbitrary write.** The export could be pointed at an arbitrary path,
  overwriting it. The payload is metadata, but it is attacker-influenced
  JSON.

**Fix.** The path parameter is gone. The Rust core opens the native file
dialog and the user picks the location:

* `src-tauri/src/commands/platform.rs` — `pick_save_path` / `pick_open_path`
  own the dialog; `export_secrets` and `read_secret_export` take no path.
* `src-tauri/src/lib.rs` — the dialog plugin is registered for the core's use.
* `src/lib/ipc.ts`, `src/screens/ImportExport.tsx` — the path field was
  removed from the UI entirely.
* The capability manifest was **not** widened: `dialog:*` is deliberately
  absent. Tauri permissions gate the webview→core direction; the webview
  never calls the dialog plugin, so granting it would only let the webview
  learn the chosen path and hand it to a future path-taking command.

**Guard.** `src-tauri/tests/security_contract.rs` asserts the commands take
no path, that `dialog:` is absent from the manifest, and that `fs:`,
`shell:allow-execute`, `shell:allow-spawn` and `http:` are absent. The
guard was verified to fail when a path parameter is reintroduced.

### F2 — No Content Security Policy
**Severity: HIGH · Status: FIXED (v1.0.0) · Category: Elevation / Tampering**

`tauri.conf.json` shipped `"csp": null`, which disables CSP entirely. In a
secrets manager the renderer is the most exposed component, and CSP is the
main defence-in-depth control against an injected script reaching a
capability.

**Fix.** A restrictive policy is now configured:

```json
"csp": {
  "default-src": "'self'",
  "connect-src": "'self' ipc: http://ipc.localhost",
  "img-src": "'self' data:",
  "font-src": "'self' data:",
  "style-src": "'self' 'unsafe-inline'",
  "script-src": "'self'",
  "object-src": "'none'",
  "base-uri": "'none'",
  "form-action": "'none'",
  "frame-ancestors": "'none'"
}
```

Notes on the choices:

* `script-src 'self'` — no inline scripts and no `eval`. React 19 in a
  production Vite build needs neither. Tauri appends the nonces and
  hashes for its own injected bootstrap code automatically.
* `connect-src` carries `ipc:` and `http://ipc.localhost` because that is
  how the webview reaches the core. No external origin is reachable.
* `style-src` keeps `'unsafe-inline'` because React inline style
  attributes are not covered by a nonce. Inline styles are not a script
  execution vector.
* `object-src`, `base-uri`, `form-action` and `frame-ancestors` are
  `'none'` — the app has no plugins, no forms and no framing.

The policy was checked against the built frontend: there are no remote
fonts, images, stylesheets or network calls in the webview. All HTTP is
issued by the Rust core.

### F3 — Plaintext reachable through `Debug`
**Severity: MEDIUM · Status: FIXED (v1.0.0) · Category: Information disclosure**

`crypto::DecryptedSecret` derived `Debug` and held a `plaintext: String`.
Any `{:?}` in a log line, error report or `panic!` would have printed the
secret verbatim.

It was also **dead code** — defined, never used.

**Fix.** The struct was deleted. Separately, `EncryptedData` — which holds
the ciphertext, the nonce and the salt — now has a hand-written `Debug`
that reports only the shape of the envelope (`key_version`, field lengths)
and never the bytes. The ciphertext, nonce and salt are the entire offline
attack surface: anyone holding all three can mount an offline passphrase
guess without touching the vault.

**Guard.** `src-tauri/tests/security_contract.rs` asserts that no
plaintext-bearing or ciphertext-bearing type derives `Debug`.

### F4 — The core's failure modes were not distinguishable from the user's
**Severity: LOW · Status: FIXED (v1.0.0) · Category: UX of a security control**

With the dialog in place, dismissing it is a normal outcome, not a fault.
Reporting it as a generic error would train users to dismiss error
dialogs.

**Fix.** A new `AppErrorKind::Cancelled` was added to the typed error
envelope, and the UI stands down silently on it.

### F5 — Idle auto-lock lived in the webview
**Severity: MEDIUM · Status: FIXED (v0.2.0, verified in this pass)**

The idle countdown runs in the Rust core, not in JavaScript. A renderer
that has been compromised can simply stop reporting activity — and the
core locks anyway. This was already correct; this review confirmed it
and that the webview's heartbeat is a coarsening of the real deadline,
not the deadline itself.

### F6 — The clipboard wipe could be skipped
**Severity: LOW · Status: FIXED (v0.2.0, verified in this pass)**

The 30-second clipboard wipe compares the clipboard against the copied
value first, so it never destroys something the user copied afterwards.
If the clipboard cannot be read at all, the wipe happens anyway — failing
to read is not evidence that it holds something else. Verified correct.

---

## Reviewed and accepted

| Control | Verdict |
| --- | --- |
| Bearer/CSRF tokens | Held in the Rust core and the OS keychain. The webview never receives them. |
| CSRF on mutations | Every mutation fetches a one-time token and sends `X-CSRF-Token`. Retries once on `csrf_token_invalid`. |
| Passphrase handling | `Zeroizing<String>` in the core; wiped on lock and on window close. Never in the webview. |
| Transport | `https://` enforced; `http://` accepted only for loopback. Non-loopback plaintext is rejected at `set_base_url`. |
| Zero-knowledge crypto | PBKDF2-HMAC-SHA256 (v1/v2) and Argon2id (v3) + AES-256-GCM. Byte-exact against the dashboard and the Go SDK. Unknown `key_version` is rejected, never guessed. |
| Decryption failure | Deliberately coarse — "wrong passphrase or corrupted payload", so an attacker learns nothing about which. |
| Platform-held secrets | Refused explicitly with `platform_held` rather than a misleading decrypt failure. |
| Auto-update | Signed with Ed25519; the verification key is compiled into the binary; `requireSignedVersion` is on. The core refuses to check without a valid key. |
| Command surface | ~70 typed commands, each a thin wrapper over one orchestrator route. No shell-out, no arbitrary URL fetch, no plugin surface beyond `updater:default`. |

---

## Residual risks

These are accepted for 1.0.0, with reasons.

| Risk | Why it is accepted |
| --- | --- |
| **No code-signing certificate.** | macOS Gatekeeper and Windows SmartScreen will warn on first launch. The certificates cost money and need an Apple Developer Program membership; the warning is a trust-marketing problem, not a vulnerability. **This is the largest remaining launch gap and needs a human decision.** |
| **`style-src` allows `'unsafe-inline'`.** | Inline styles are not a script-execution vector, and React uses them. Tightening it would need a nonce-aware style pipeline for no security gain. |
| **The orchestrator sees ciphertext and metadata.** | That is the zero-knowledge design. Metadata leakage (secret names, namespaces, rotation times) is inherent to a centrally-indexed vault. |
| **The unlock passphrase derives the key directly.** | No wrapping key, no hardware-backed key. An attacker with both the ciphertext and the passphrase has the secret. Argon2id (v3) raises the cost of an offline guess; a passphrase-based unlock is the product's deliberate simplicity trade. |
| **The audit chain may be unkeyed.** | The chain panel distinguishes keyed (tamper-evident) from unkeyed (corruption detection only) and never claims more than the server reports. A tenant that needs tamper-evidence must configure the chain key server-side. |
| **No branch protection.** | The GitHub free plan rejects it for private repositories. Manual review is the only merge gate today. |
| **GitHub Actions is down org-wide.** | The release pipeline cannot run in CI. It is executed by hand from the build host, which is slower and less reproducible. |

---

## Rules that follow from this review

1. **Do not widen `src-tauri/capabilities/default.toml`** without repeating
   this review and updating `src-tauri/tests/security_contract.rs`.
2. **The webview must never name a path.** If a command needs to touch the
   filesystem, it opens a dialog itself and takes no path.
3. **No type that holds plaintext or ciphertext may derive `Debug`.**
4. **The webview makes no network calls.** Every request goes through the
   core, so the bearer never reaches JavaScript.
5. The idle timer, the clipboard wipe and the unlock state live in the
   **core**, not the renderer. A compromised renderer must not be able to
   talk the app out of them.
