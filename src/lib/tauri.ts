// Thin wrapper around `@tauri-apps/api/core::invoke`. We use a small wrapper
// (not the raw `invoke`) so the rest of the app never imports a Tauri
// symbol directly — this gives us a single place to mock the IPC in tests.
//
// We deliberately do NOT catch IPC errors here; the structured AppError
// returned by the Rust side is already typed.
//
// IMPORTANT: the `@tauri-apps/api/core` import is intentionally done lazily
// via `await import(...)` so the module loads cleanly under vitest (no Tauri
// host present). The bare static import would otherwise hang the test runner
// while it tries to detect a host that does not exist.

export function isTauri(): boolean {
  return typeof (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
}

export interface AppErrorShape {
  kind:
    | "unauthenticated"
    | "vault_locked"
    | "invalid_credentials"
    | "rate_limited"
    | "plan_denied"
    | "not_found"
    | "csrf_failed"
    | "invalid_input"
    | "storage"
    | "transport"
    | "unknown";
  message: string;
}

export class VaultIpcError extends Error {
  readonly kind: AppErrorShape["kind"];
  constructor(shape: AppErrorShape) {
    super(shape.message);
    this.name = "VaultIpcError";
    this.kind = shape.kind;
  }
}

// Look up the Tauri `invoke` on every call so the module is hot-swappable
// under `vi.resetModules()` (the test harness resets the module registry
// between cases; module-level caches would otherwise pin the first test's
// mock across all later tests). This also means production never pays for
// the import unless `invoke()` is actually called from inside the Tauri
// runtime — which it always is, so the overhead is one ESM lookup per call.
type TauriInvoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
async function realTauriInvoke(): Promise<TauriInvoke> {
  const mod = await import("@tauri-apps/api/core");
  return mod.invoke as TauriInvoke;
}

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri()) {
    throw new Error(
      `Tauri IPC is unavailable (cmd=${cmd}). The webview must be hosted by the Tauri runtime.`
    );
  }
  const tauriInvoke = await realTauriInvoke();
  try {
    return await tauriInvoke<T>(cmd, args);
  } catch (raw) {
    // Tauri v2 may reject with either the parsed object (preferred) or an
    // Error whose `.message` is the JSON string. Handle both shapes; if
    // neither contains {kind, message}, fall back to a plain Error.
    const parsed = extractAppError(raw);
    if (parsed) {
      throw new VaultIpcError(parsed);
    }
    throw new Error(
      `Tauri IPC error (cmd=${cmd}): ${raw instanceof Error ? raw.message : String(raw)}`
    );
  }
}

function extractAppError(raw: unknown): AppErrorShape | null {
  if (raw && typeof raw === "object" && "kind" in raw && "message" in raw) {
    return raw as AppErrorShape;
  }
  if (raw instanceof Error && typeof raw.message === "string") {
    try {
      const inner = JSON.parse(raw.message);
      if (inner && typeof inner === "object" && "kind" in inner && "message" in inner) {
        return inner as AppErrorShape;
      }
    } catch {
      // not JSON; fall through
    }
  }
  return null;
}
