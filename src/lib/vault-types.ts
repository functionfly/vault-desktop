// Types for the vault API surface the desktop client reaches.
//
// These mirror the orchestrator's own response shapes. Where the server is
// the authority, the comment names the Go type it comes from, so a change on
// the server can be traced to the exact struct to compare against.

// ── Audit hash chain ──────────────────────────────────────────────────────

/**
 * One segment of a tenant's audit hash chain.
 *
 * Mirrors `vault.AuditChainSegmentStatus`.
 */
export interface AuditChainSegment {
  segment: number;
  entries: number;
  genesis_hash?: string;
  head_hash?: string;
  anchored: boolean;
  /** True when the segment's anchor still verifies under the chain key. */
  anchor_valid: boolean;
  updated_at?: string;
}

/**
 * The chain's posture and shape.
 *
 * Mirrors `vault.AuditChainStatus`. `tamper_evident` is FALSE whenever the
 * chain is unkeyed: an unkeyed chain detects accidental corruption but not
 * a deliberate rewrite. The UI must not claim otherwise.
 */
export interface AuditChainStatus {
  keyed: boolean;
  posture: "keyed" | "unkeyed";
  tamper_evident: boolean;
  total_entries: number;
  active_segment: number;
  segments: AuditChainSegment[];
}

/** The first entry that did not verify, when the chain is broken. */
export interface AuditChainBreak {
  id: string;
  segment: number;
  position: number;
  reason: string;
}

/**
 * The result of verifying a tenant's audit chain.
 *
 * Mirrors `vault.AuditChainVerifyResponse`. The server answers 200 whether
 * or not the chain verified — a break is a security finding, not a client
 * error — so the caller must inspect `intact`.
 */
export interface AuditChainVerifyResult {
  intact: boolean;
  keyed: boolean;
  tamper_evident: boolean;
  /** True when only the most recent entries were re-hashed. */
  windowed: boolean;
  /** How many entries were re-hashed (0 = the whole chain). */
  window: number;
  total: number;
  verified: number;
  legacy: number;
  segments: number;
  anchored_segments: number;
  unanchored_segments: number;
  break: AuditChainBreak | null;
  checked_at: string;
}

// ── Capability grants ─────────────────────────────────────────────────────

/**
 * Whether the platform can resolve a secret's plaintext.
 *
 * A `zk_mode: false` secret is platform-held. It is the only kind a
 * capability "use" grant can serve, because the broker has to resolve the
 * value at dispatch.
 */
export interface SecretPresence {
  zk_mode: boolean;
  /** True when the platform can resolve the secret's plaintext. */
  present: boolean;
  /** The secret's current version, when the server reports one. */
  current_version?: number;
}

// ── Tenant vault MFA ──────────────────────────────────────────────────────

/**
 * The tenant's vault MFA policy.
 *
 * Mirrors the server's `mfaConfigResponse`.
 */
export interface MfaConfig {
  mfa_required: boolean;
  /** `totp`, or whatever the tenant configured. */
  mfa_method: string;
  /** How long a verified assertion lasts, in seconds. */
  session_ttl_seconds?: number;
}

/** The result of verifying a vault MFA code. */
export interface MfaVerifyResult {
  verified: boolean;
  expires_at: string;
  ttl: number;
}

// ── Dynamic credentials ───────────────────────────────────────────────────

/** The database a dynamic target points at. */
export type DynamicDbType = "postgres" | "mysql";

export interface CreateDynamicTargetInput {
  name: string;
  description?: string;
  db_type: DynamicDbType;
  host: string;
  port: number;
  database_name: string;
  admin_username: string;
  /** Plaintext password; encrypted server-side. Never returned by the API. */
  admin_password: string;
  ssl_mode?: string;
  allowed_roles?: string[];
  default_ttl_seconds?: number;
  max_ttl_seconds?: number;
}

export interface CreateDynamicCredentialInput {
  target_id: string;
  name: string;
  description?: string;
  role_template?: string;
  ttl_seconds?: number;
  max_ttl_seconds?: number;
}

/**
 * A freshly minted credential and its lease.
 *
 * The username and password exist only in this response — the platform does
 * not store them — so the UI must show them once and never persist them.
 */
export interface GeneratedCredential {
  lease_id: string;
  username: string;
  password: string;
  host: string;
  port: number;
  database: string;
  expires_at: string;
}

// ── Sharing and federation ────────────────────────────────────────────────

export interface ShareSecretInput {
  secret_id: string;
  /** The tenant member to share with. */
  user_id: string;
  scopes?: string[];
  expires_at?: string;
}

export interface CreateFederationKeyInput {
  name: string;
  description?: string;
  peer_tenant_id?: string;
  scopes?: string[];
  expires_at?: string;
}

// ── Import / export ───────────────────────────────────────────────────────

/** The result of writing an export to disk. */
export interface SecretExportResult {
  /**
   * Where the export landed.
   *
   * The path is chosen by the user in the native save dialog, which the
   * Rust core owns. The webview never supplies it.
   */
  path: string;
  count: number;
}

/**
 * A parsed export file.
 *
 * The export is METADATA only. It carries no ciphertext and no plaintext.
 */
export interface SecretExportSummary {
  format_version: number;
  exported_at: string;
  base_url: string;
  tenant_id: string;
  count: number;
  secrets: Array<Record<string, unknown>>;
}

// ── Offline metadata cache ───────────────────────────────────────────────

/** One cached secret's metadata. */
export interface CachedSecretMetadata {
  id: string;
  name: string;
  description: string;
  secret_type: string;
  namespace: string;
  scopes: string[];
  /** False means the platform holds this secret. */
  zk_mode: boolean;
  access_count: number;
  created_at: string;
  updated_at: string;
}

/**
 * The offline metadata cache.
 *
 * `available` is false when there is no cache yet, or when the cache
 * belongs to a different orchestrator or tenant — a cache from one
 * tenant must never be shown against another.
 */
export interface MetadataCacheResult {
  available: boolean;
  reason?: string;
  cached_at?: string;
  base_url?: string;
  tenant_id?: string;
  count?: number;
  secrets?: CachedSecretMetadata[];
}

// ── Idle auto-lock ────────────────────────────────────────────────────────

export interface IdleSettings {
  /** The configured timeout in seconds. `0` means auto-lock is off. */
  timeout_seconds: number;
  /** The selectable values, in seconds. */
  choices: number[];
}
