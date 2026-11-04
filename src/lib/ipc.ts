// Typed wrappers around every Tauri command. These are the only place the
// webview calls `invoke` for vault operations, so the rest of the UI can
// use normal async functions without knowing about the IPC layer.

import { invoke } from "./tauri";

export interface LoginUser {
  id: string;
  email?: string | null;
  username?: string | null;
  tenant_id: string;
  role?: string | null;
}

export interface LoginResult {
  user: LoginUser;
  desktop_capable: boolean;
}

export interface RestoreResult {
  user?: LoginUser;
  refreshed: boolean;
  base_url?: string;
}

export interface VaultStatus {
  authenticated: boolean;
  unlocked: boolean;
  /** Which orchestrator this session is bound to. */
  base_url?: string;
  /** Where the long-lived token lives: `os_keychain`, `file_fallback`, or `empty`. */
  store_backend?: StoreBackend;
  user?: LoginUser;
}

/** Which credential backend is live. `file_fallback` means the host has no
 *  reachable OS keyring and the token is sealed under a per-install AES-GCM
 *  key in the platform data dir — still encrypted at rest, but weaker than a
 *  real keyring, so the UI surfaces it. */
export type StoreBackend = "os_keychain" | "file_fallback" | "empty";

/** Result of the login screen's "Test connection" probe. */
export interface HealthProbe {
  reachable: boolean;
  latency_ms: number;
  version?: string | null;
  /** Human-readable explanation. Populated when `reachable` is false. */
  detail?: string | null;
}

/** What the app knows about its own credential store before sign-in. */
export interface StoreInfo {
  backend: StoreBackend;
  has_session: boolean;
  stored_base_url: string;
}

export interface ListQuery {
  search?: string;
  namespace?: string;
  secret_type?: string;
  limit?: number;
  offset?: number;
}

export interface ListSecretsResult {
  secrets: Array<Record<string, unknown>>;
  total: number;
}

export interface RevealResult {
  secret_id: string;
  plaintext: string;
}

// ── write path ─────────────────────────────────────────────────────────────
//
// The plaintext NEVER leaves the Rust core in encrypted form: these inputs
// carry the raw value, the core encrypts it under the unlock passphrase, and
// only the resulting `encrypted_data` payload is sent to the orchestrator.

export interface CreateSecretInput {
  name: string;
  description?: string;
  secret_type: string;
  plaintext: string;
  namespace?: string;
  scopes?: string[];
}

export interface UpdateSecretInput {
  id: string;
  name?: string;
  description?: string;
  scopes?: string[];
}

export interface RotateSecretInput {
  id: string;
  plaintext: string;
  reason?: string;
}

/**
 * What the login screen shows while waiting for the human to approve a
 * browser sign-in. Mirrors the Rust `DeviceCodePrompt` — the `device_code`
 * that authorises the token exchange never crosses the IPC boundary, so a
 * compromised renderer cannot complete the exchange on its own.
 */
export interface DeviceCodePrompt {
  /** Human-typed code, e.g. "WXYZ-2345". */
  user_code: string;
  /** Where to send the human, without a code pre-filled. */
  verification_uri: string;
  /** Same URL with the code pre-filled — prefer this for the "open browser" button. */
  verification_uri_complete: string;
  /** Seconds until the grant expires. */
  expires_in: number;
  /** Seconds the client should wait between polls. */
  interval: number;
}

export const ipc = {
  login: (baseUrl: string, email: string, password: string) =>
    invoke<LoginResult>("login", { baseUrl, email, password }),

  loginWithApiKey: (baseUrl: string, apiKey: string) =>
    invoke<LoginResult>("login_with_api_key", { baseUrl, apiKey }),

  /**
   * Starts a browser sign-in (RFC 8628 device grant). Returns the code the
   * human must approve — no credential is exchanged yet.
   */
  beginDeviceLogin: (baseUrl: string) =>
    invoke<DeviceCodePrompt>("begin_device_login", { baseUrl }),

  /**
   * Blocks until the human approves, the grant expires, or it is denied, then
   * persists the session. One call rather than a poll-per-click so the
   * backoff rules stay in Rust where the timing lives.
   */
  awaitDeviceLogin: (userCode: string) =>
    invoke<LoginResult>("await_device_login", { userCode }),

  restoreSession: () => invoke<RestoreResult>("restore_session"),

  logout: () => invoke<void>("logout"),

  unlockVault: (passphrase: string) =>
    invoke<void>("unlock_vault", { passphrase }),
  lockVault: () => invoke<void>("lock_vault"),
  vaultStatus: () => invoke<VaultStatus>("vault_status"),

  /** Probes an orchestrator URL before signing in. Sends no credential. */
  healthProbe: (baseUrl: string) =>
    invoke<HealthProbe>("vault_health_probe", { baseUrl }),

  /** Reports the live credential backend so the login screen can prefill the
   *  last-used URL and warn when the token is only file-encrypted. */
  storeInfo: () => invoke<StoreInfo>("vault_store_info"),

  listSecrets: (query: ListQuery) =>
    invoke<ListSecretsResult>("list_secrets", { query }),

  getSecret: (id: string) =>
    invoke<Record<string, unknown>>("get_secret", { id }),

  listSecretVersions: (id: string) =>
    invoke<Record<string, unknown>>("list_secret_versions", { id }),

  revealSecret: (id: string) => invoke<RevealResult>("reveal_secret", { id }),

  diffSecretVersions: (id: string, from: number, to: number) =>
    invoke<Record<string, unknown>>("diff_secret_versions", { id, from, to }),

  rollbackSecret: (id: string, targetVersion: number, reason?: string) =>
    invoke<Record<string, unknown>>("rollback_secret", {
      id,
      targetVersion,
      reason: reason ?? "",
    }),

  createSecret: (input: CreateSecretInput) =>
    invoke<Record<string, unknown>>("create_secret", { input }),

  updateSecret: (input: UpdateSecretInput) =>
    invoke<Record<string, unknown>>("update_secret", { input }),

  rotateSecret: (input: RotateSecretInput) =>
    invoke<Record<string, unknown>>("rotate_secret", { input }),

  deleteSecret: (id: string) => invoke<void>("delete_secret", { id }),

  listNamespaces: () => invoke<Record<string, unknown>>("list_namespaces"),
  createNamespace: (name: string) =>
    invoke<Record<string, unknown>>("create_namespace", { name }),
  deleteNamespace: (id: string) => invoke<void>("delete_namespace", { id }),

  // ── Tokens ─────────────────────────────────────────────────────────────
  // Per-secret short-lived access grants. The orchestrator returns the
  // plaintext exactly once at mint; thereafter only the SHA-256 hash is
  // stored server-side.
  listTokens: (secretId: string) =>
    invoke<Record<string, unknown>>("list_tokens", { id: secretId }),

  generateToken: (input: GenerateTokenInput) =>
    invoke<Record<string, unknown>>("generate_token", { input }),

  revokeToken: (tokenId: string) => invoke<void>("revoke_token", { tokenId }),

  // ── Bulk operations ───────────────────────────────────────────────────
  // dryRun=true returns a per-secret dependency map; dryRun=false actually
  // deletes. The orchestrator caps at 100 IDs per request.
  bulkDeleteSecrets: (input: BulkDeleteInput) =>
    invoke<Record<string, unknown>>("bulk_delete_secrets", { input }),

  // ── Audit (filtered) ──────────────────────────────────────────────────
  // The base `list_audit` returns the full audit log. These two wrappers
  // reach the orchestrator's per-action / per-actor filter paths so the
  // AuditLog screen can drill into "everything secret.read for the last
  // 7 days" without scrolling through unrelated entries.
  listAuditByAction: (action: string, limit?: number) =>
    invoke<Record<string, unknown>>("list_audit_by_action", {
      action,
      limit,
    }),

  listAuditByActor: (actorType: string, actorId: string, limit?: number) =>
    invoke<Record<string, unknown>>("list_audit_by_actor", {
      actorType,
      actorId,
      limit,
    }),

  listAudit: (secretId?: string, limit?: number) =>
    invoke<Record<string, unknown>>("list_audit", { secretId, limit }),

  // ── Updater (signed auto-update; fail-closed) ──────────────────────────
  // The Rust core refuses to check for updates without a valid Ed25519
  // pubkey. It is compiled into the binary from
  // `plugins.updater.pubkey`; `VAULT_UPDATER_PUBKEY` is an operator
  // override that takes precedence. See
  // `src-tauri/src/updater.rs::UpdaterConfig::from_env` for the
  // contract. When no key is available at all, the response is a typed
  // `{ code: "missing_pubkey", ... }` envelope rather than a stack
  // trace so the React UI can render a clear "updater disabled"
  // state.
  checkUpdate: (channel: UpdaterChannel) =>
    invoke<CheckUpdateResult>("vault_check_update", { channel }),

  applyUpdate: (channel: UpdaterChannel) =>
    invoke<ApplyUpdateResult>("vault_apply_update", { channel }),
} as const;

// ── Input shapes ────────────────────────────────────────────────────────
//
// `CreateSecretInput`, `UpdateSecretInput` and `RotateSecretInput` are
// declared once, in the "write path" section above. They were previously
// declared twice — TypeScript silently merged the two declarations, so
// the effective type accepted fields (`expires_at` on create,
// `namespace` on update) that the Rust command structs never read. A
// caller could set them and watch them vanish without any error.
//
// The shapes below mirror the Rust command structs exactly:
//
//   CreateSecretInput  { name, description, secret_type, plaintext,
//                        namespace, scopes }
//   UpdateSecretInput  { id, name, description, scopes }
//   RotateSecretInput  { id, plaintext, reason }

export interface GenerateTokenInput {
  id: string;
  name: string;
  scopes: string[];
  expires_in_hours: number;
}

export interface BulkDeleteInput {
  secret_ids: string[];
  dry_run: boolean;
}

// ── Updater types ───────────────────────────────────────────────────────

/** Channels the updater understands. Mirrors `UpdaterChannel` in
 * `src-tauri/src/updater.rs`. Kept as a string union so the webview
 * doesn't need to import the Rust crate.
 */
export type UpdaterChannel = "stable" | "beta";

/** Result envelope from `vault_check_update`. Discriminated on
 * `result` so the React UI can branch without parsing prose. Mirrors
 * the `CheckUpdateResponse` enum in `src-tauri/src/commands/updater.rs`.
 */
export type CheckUpdateResult =
  | { result: "up_to_date" }
  | {
      result: "available";
      version: string;
      notes: string;
      pub_date: string | null;
    };

/** Result envelope from `vault_apply_update`. Mirrors the
 * `ApplyUpdateResponse` enum in `src-tauri/src/commands/updater.rs`.
 */
export interface ApplyUpdateResult {
  installed: { version: string };
}

/** Typed error envelope from the updater commands. The webview
 * branches on `code` rather than parsing the `message` prose. Mirrors
 * the `VaultUpdateError` struct in `src-tauri/src/commands/updater.rs`.
 */
export interface UpdaterErrorShape {
  code:
    | "missing_pubkey"
    | "invalid_endpoint"
    | "missing_endpoint"
    | "unknown_channel"
    | "builder_error"
    | "check_failed"
    | "install_failed"
    | "no_update_available";
  message: string;
}
