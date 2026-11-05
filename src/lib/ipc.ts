// Typed wrappers around every Tauri command. These are the only place the
// webview calls `invoke` for vault operations, so the rest of the UI can
// use normal async functions without knowing about the IPC layer.

import { invoke } from "./tauri";
import type {
  AuditChainStatus,
  AuditChainVerifyResult,
  CreateDynamicCredentialInput,
  CreateDynamicTargetInput,
  CreateFederationKeyInput,
  GeneratedCredential,
  IdleSettings,
  MfaConfig,
  MfaVerifyResult,
  MetadataCacheResult,
  SecretExportResult,
  SecretExportSummary,
  SecretPresence,
  ShareSecretInput,
} from "./vault-types";

export type {
  AuditChainBreak,
  AuditChainSegment,
  AuditChainStatus,
  AuditChainVerifyResult,
  CachedSecretMetadata,
  CreateDynamicCredentialInput,
  CreateDynamicTargetInput,
  CreateFederationKeyInput,
  DynamicDbType,
  GeneratedCredential,
  IdleSettings,
  MfaConfig,
  MfaVerifyResult,
  MetadataCacheResult,
  SecretExportResult,
  SecretExportSummary,
  SecretPresence,
  ShareSecretInput,
} from "./vault-types";

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
  /**
   * Ask the PLATFORM to hold this secret instead of encrypting it here.
   *
   * This WIDENS the trust boundary — the platform can then read the secret —
   * so it is opt-in and the orchestrator gates it on the ADMIN tenant role.
   * It is also the only kind of secret a capability "use" grant can serve:
   * a zero-knowledge secret can never back one.
   */
  server_side?: boolean;
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

  // ── Audit hash chain (tamper-evidence) ─────────────────────────────────
  // The chain is a per-tenant hash chain: each entry commits to its own
  // content and to the previous entry's hash. `getAuditChain` reports the
  // posture and the segments; `verifyAuditChain` re-hashes and reports the
  // first entry that does not verify.
  //
  // IMPORTANT: the server answers 200 whether or not the chain verified — a
  // break is a security finding, not a client error. So the caller must
  // inspect `intact`, not treat a resolved promise as a clean audit.
  getAuditChain: () =>
    invoke<AuditChainStatus>("get_audit_chain"),

  verifyAuditChain: (window?: number) =>
    invoke<AuditChainVerifyResult>("verify_audit_chain", { window }),

  // ── Capability grants ─────────────────────────────────────────────────
  // A capability request asks for scoped access to a secret. Only a
  // PLATFORM-HELD secret can back one: a zero-knowledge secret has no
  // ciphertext the broker could deliver.
  listCapabilities: () =>
    invoke<Record<string, unknown>>("list_capabilities"),
  approveCapability: (id: string) =>
    invoke<Record<string, unknown>>("approve_capability", { id }),
  denyCapability: (id: string) =>
    invoke<Record<string, unknown>>("deny_capability", { id }),
  revokeCapability: (id: string) =>
    invoke<Record<string, unknown>>("revoke_capability", { id }),

  /** Whether the platform can resolve this secret's plaintext. */
  getSecretPresence: (id: string) =>
    invoke<SecretPresence>("get_secret_presence", { id }),

  // ── Tenant vault MFA ──────────────────────────────────────────────────
  // The MFA code travels in a header, never in a query string.
  getMfaConfig: () => invoke<MfaConfig>("get_mfa_config"),
  verifyMfa: (code: string) => invoke<MfaVerifyResult>("verify_mfa", { code }),

  // ── Dynamic credentials ───────────────────────────────────────────────
  createDynamicTarget: (input: CreateDynamicTargetInput) =>
    invoke<Record<string, unknown>>("create_dynamic_target", { input }),
  listDynamicTargets: () =>
    invoke<Record<string, unknown>>("list_dynamic_targets"),
  deleteDynamicTarget: (id: string) =>
    invoke<void>("delete_dynamic_target", { id }),
  testDynamicTarget: (id: string) =>
    invoke<Record<string, unknown>>("test_dynamic_target", { id }),

  createDynamicCredential: (input: CreateDynamicCredentialInput) =>
    invoke<Record<string, unknown>>("create_dynamic_credential", { input }),
  listDynamicCredentials: () =>
    invoke<Record<string, unknown>>("list_dynamic_credentials"),
  /** Mints a live credential. The response carries the plaintext once. */
  generateDynamicCredential: (id: string, ttlSeconds?: number) =>
    invoke<GeneratedCredential>("generate_dynamic_credential", {
      id,
      ttlSeconds,
    }),
  revokeDynamicCredential: (id: string) =>
    invoke<void>("revoke_dynamic_credential", { id }),
  renewLease: (credentialId: string, leaseId: string, ttlSeconds?: number) =>
    invoke<Record<string, unknown>>("renew_lease", {
      credentialId,
      leaseId,
      ttlSeconds,
    }),
  revokeLease: (credentialId: string, leaseId: string) =>
    invoke<void>("revoke_lease", { credentialId, leaseId }),

  // ── Sharing and federation ────────────────────────────────────────────
  shareSecret: (input: ShareSecretInput) =>
    invoke<Record<string, unknown>>("share_secret", { input }),
  listSharedSecrets: () => invoke<Record<string, unknown>>("list_shared_secrets"),
  revokeShare: (shareId: string) => invoke<void>("revoke_share", { shareId }),

  listFederationKeys: () => invoke<Record<string, unknown>>("list_federation_keys"),
  createFederationKey: (input: CreateFederationKeyInput) =>
    invoke<Record<string, unknown>>("create_federation_key", { input }),

  // ── Import / export (metadata only) ───────────────────────────────────
  // The export carries secret METADATA — never a ciphertext, never a
  // plaintext. The file is written by the Rust core so the webview never
  // needs a filesystem capability.
  //
  // Neither command takes a path. The core opens the native file dialog
  // and the user picks the location, so a compromised renderer cannot
  // direct the core at an arbitrary file.
  exportSecrets: () => invoke<SecretExportResult>("export_secrets"),
  readSecretExport: () =>
    invoke<SecretExportSummary>("read_secret_export"),

  // ── Offline metadata cache ────────────────────────────────────────────
  // The cache holds secret METADATA only — never a ciphertext, never a
  // plaintext. It lets the app show what you have while the orchestrator
  // is unreachable.
  readMetadataCache: () =>
    invoke<MetadataCacheResult>("read_metadata_cache"),
  clearMetadataCache: () => invoke<void>("clear_metadata_cache"),

  // ── Idle auto-lock ────────────────────────────────────────────────────
  // The countdown lives in the Rust core; the webview only reports
  // activity. A compromised renderer cannot suppress the timer by simply
  // not running it.
  reportActivity: () => invoke<void>("report_activity"),
  getIdleSettings: () => invoke<IdleSettings>("get_idle_settings"),
  setIdleSettings: (timeoutSeconds: number) =>
    invoke<IdleSettings>("set_idle_settings", { timeoutSeconds }),
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
