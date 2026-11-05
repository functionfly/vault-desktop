/**
 * A browser-only mock of the Tauri IPC surface.
 *
 * WHY THIS EXISTS
 * ---------------
 * The desktop app is a Tauri application: its UI is a React webview that
 * talks to a Rust core over an IPC channel. Opened in a plain browser
 * there is no host, so `isTauri()` is false and every call throws
 * "Tauri IPC is unavailable" — the page renders nothing.
 *
 * That makes the UI impossible to screenshot, document, or review in a
 * browser, and it means a reviewer cannot see the interface at all
 * without installing the desktop app.
 *
 * This mock stands in for the Rust core when the app is loaded in a
 * browser, so the interface can be rendered, captured and reviewed. It
 * answers the same commands the core answers, with the same shapes.
 *
 * IT IS NEVER BUNDLED INTO THE SHIPPED APP.
 *
 * The mock is installed only when `?mock=1` is in the URL AND the page
 * is not running inside Tauri. The real app therefore never loads it,
 * and the shipped bundle contains no mock code path at all — the
 * bundler drops it, because `import.meta.env.DEV` is false in a
 * production build and the whole module is behind a dynamic import.
 *
 * The data is fictional. No real tenant, no real secret, no real
 * credential.
 */

import type { LoginUser } from "./ipc";

/** The fake tenant the mock pretends to be signed in as. */
const MOCK_USER: LoginUser = {
  id: "user_demo_001",
  email: "demo@example.com",
  tenant_id: "tenant_demo",
  role: "admin",
};

/** A fictional secret. The values are obviously fake on purpose. */
const MOCK_SECRETS = [
  {
    id: "sec_01",
    name: "production-database",
    secret_type: "generic",
    namespace: "production",
    description: "Primary Postgres connection string",
    scopes: ["read", "write"],
    version: 7,
    updated_at: "2026-11-04T18:20:00Z",
  },
  {
    id: "sec_02",
    name: "stripe-live-api-key",
    secret_type: "api_key",
    namespace: "billing",
    description: "Stripe secret key for the billing service",
    scopes: ["read"],
    version: 3,
    updated_at: "2026-11-03T09:05:00Z",
  },
  {
    id: "sec_03",
    name: "smtp-relay-password",
    secret_type: "password",
    namespace: "email",
    description: "SMTP relay credential for transactional mail",
    scopes: ["read", "rotate"],
    version: 12,
    updated_at: "2026-10-29T22:41:00Z",
  },
  {
    id: "sec_04",
    name: "github-actions-deploy-key",
    secret_type: "ssh_key",
    namespace: "ci",
    description: "Deploy key for the release pipeline",
    scopes: ["read"],
    version: 1,
    updated_at: "2026-10-21T11:12:00Z",
  },
];

/** The commands the mock answers, keyed by command name. */
type Handler = (args: Record<string, unknown>) => unknown;

const HANDLERS: Record<string, Handler> = {
  // ── Session ───────────────────────────────────────────────────────────
  restore_session: () => ({
    user: MOCK_USER,
    refreshed: false,
    base_url: "https://api.functionfly.com",
  }),
  vault_status: () => ({
    authenticated: true,
    // The mock reports the vault as UNLOCKED so the browser renders the
    // secret list, the audit log and the other vault surfaces — not just
    // the unlock screen. The desktop app is the authority on this; the
    // mock only has to look like a session that has been unlocked.
    unlocked: true,
    base_url: "https://api.functionfly.com",
    store_backend: "os_keychain",
    user: MOCK_USER,
  }),
  store_info: () => ({
    backend: "os_keychain",
    location: "Keychain / Credential Manager / libsecret",
  }),
  health_probe: () => ({ reachable: true, latency_ms: 42 }),
  logout: () => null,
  lock_vault: () => null,
  report_activity: () => null,

  // ── Login ─────────────────────────────────────────────────────────────
  login: () => ({ user: MOCK_USER, desktop_capable: true }),
  login_with_api_key: () => ({ user: MOCK_USER, desktop_capable: true }),
  begin_device_login: () => ({
    user_code: "WXYZ-1234",
    verification_uri: "https://app.functionfly.com/device",
    expires_in: 900,
  }),
  await_device_login: () => ({ user: MOCK_USER, desktop_capable: true }),
  unlock_vault: () => null,

  // ── Secrets ───────────────────────────────────────────────────────────
  list_secrets: (args) => {
    const query = (args?.query ?? {}) as { search?: string };
    const search = (query.search ?? "").toLowerCase();
    const secrets = search
      ? MOCK_SECRETS.filter((s) =>
          [s.name, s.description, s.namespace, s.secret_type]
            .join(" ")
            .toLowerCase()
            .includes(search),
        )
      : MOCK_SECRETS;
    return { secrets, total: secrets.length };
  },
  get_secret: () => MOCK_SECRETS[0],
  list_secret_versions: () => ({
    versions: [{ version: 7, created_at: "2026-11-04T18:20:00Z" }],
    total: 1,
  }),
  list_namespaces: () => ({
    namespaces: [
      { id: "ns_prod", name: "production" },
      { id: "ns_bill", name: "billing" },
      { id: "ns_email", name: "email" },
      { id: "ns_ci", name: "ci" },
    ],
    total: 4,
  }),

  // ── Audit ─────────────────────────────────────────────────────────────
  list_audit: () => ({
    entries: [
      {
        id: "ae_1",
        action: "secret.read",
        actor_type: "user",
        actor_id: "demo@example.com",
        target: "production-database",
        created_at: "2026-11-04T18:22:10Z",
      },
      {
        id: "ae_2",
        action: "secret.rotate",
        actor_type: "user",
        actor_id: "demo@example.com",
        target: "smtp-relay-password",
        created_at: "2026-11-03T09:04:55Z",
      },
    ],
    total: 2,
  }),
  list_audit_by_action: () => ({ entries: [], total: 0 }),
  list_audit_by_actor: () => ({ entries: [], total: 0 }),

  // ── Platform surfaces ─────────────────────────────────────────────────
  get_audit_chain: () => ({
    keyed: true,
    posture: "keyed",
    tamper_evident: true,
    total_entries: 4821,
    active_segment: 3,
    segments: [],
  }),
  verify_audit_chain: () => ({
    intact: true,
    keyed: true,
    tamper_evident: true,
    windowed: false,
    window: 0,
    total: 4821,
    verified: 4821,
    legacy: 0,
    segments: 3,
    anchored_segments: 3,
    unkeyed_segments: 0,
    break: null,
    checked_at: "2026-11-05T00:00:00Z",
  }),
  list_capabilities: () => ({ capabilities: [], total: 0 }),
  get_secret_presence: () => ({ zk_mode: true, present: true, current_version: 7 }),
  get_mfa_config: () => ({ mfa_required: false, method: "totp" }),
  verify_mfa: () => ({ valid: true }),
  list_dynamic_targets: () => ({ targets: [], total: 0 }),
  list_dynamic_credentials: () => ({ credentials: [], total: 0 }),
  list_shared_secrets: () => ({ secrets: [], total: 0 }),
  list_federation_keys: () => ({ keys: [], total: 0 }),
  list_tokens: () => ({ tokens: [], total: 0 }),
  read_metadata_cache: () => ({ secrets: [], total: 0, cached_at: null }),
  clear_metadata_cache: () => null,
  get_idle_settings: () => ({ timeout_seconds: 300, choices: [0, 60, 300, 900, 1800] }),
  set_idle_settings: () => ({ timeout_seconds: 300 }),

  // ── Updater ───────────────────────────────────────────────────────────
  vault_check_update: () => ({ result: "up_to_date" }),
  vault_apply_update: () => ({ installed: { version: "1.0.0" } }),
};

/**
 * Installs the mock into `window.__TAURI_INTERNALS__`.
 *
 * The real `@tauri-apps/api` calls exactly one function on that object:
 *
 *   window.__TAURI_INTERNALS__.invoke(cmd, args, options)
 *
 * plus `transformCallback` for the event/callback plumbing. The mock
 * provides both.
 */
export function installMock(): void {
  const internals = {
    transformCallback: (callback: (payload: unknown) => void, once = false) => {
      const id = Math.floor(Math.random() * 1_000_000);
      const registry = (globalThis as { __MOCK_CALLBACKS__?: Map<number, (p: unknown) => void> })
        .__MOCK_CALLBACKS__ ??= new Map();
      registry.set(id, (payload: unknown) => {
        callback(payload);
        if (once) registry.delete(id);
      });
      return id;
    },
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      const handler = HANDLERS[cmd];
      if (!handler) {
        // An unknown command is a real gap in the mock, not a reason to
        // fail the page — but it should be visible.
        console.warn(`[mock] no handler for command: ${cmd}`);
        return null;
      }
      // A small delay so loading states are visible in a screenshot.
      await new Promise((resolve) => setTimeout(resolve, 40));
      return handler(args);
    },
    convertFileSrc: (path: string) => path,
  };

  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
}

/** True when the URL asked for the mock and the app is not in Tauri. */
export function shouldInstallMock(): boolean {
  if (typeof globalThis === "undefined") return false;
  const inTauri =
    typeof (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    "undefined";
  if (inTauri) return false;
  if (typeof location === "undefined") return false;
  return new URLSearchParams(location.search).get("mock") === "1";
}
