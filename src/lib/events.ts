// Tauri event subscription.
//
// The `@tauri-apps/api/event` import is done lazily for the same reason
// `@tauri-apps/api/core` is: a static import would try to detect a Tauri
// host at module load, which hangs under vitest where there is none.

import { isTauri } from "./tauri";

type Unlisten = () => void;

/**
 * Subscribes to a Tauri event.
 *
 * Returns a no-op unlisten when there is no Tauri host, so callers never
 * have to branch on the environment.
 */
export async function listen(
  event: string,
  handler: (payload: unknown) => void,
): Promise<Unlisten> {
  if (!isTauri()) {
    return () => {};
  }
  const mod = await import("@tauri-apps/api/event");
  const unlisten = await mod.listen(event, (e) => handler(e.payload));
  return unlisten;
}

/** Emitted by the Rust core when the vault auto-locks after an idle period. */
export const EVENT_VAULT_LOCKED = "vault://locked";
