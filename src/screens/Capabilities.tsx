// Capability grants: scoped access requests against platform-held secrets.
//
// A capability request is someone asking to be granted access to a secret
// within a scope. The grant is only SERVICEABLE for a platform-held secret
// — a zero-knowledge secret has nothing for the broker to deliver — which
// is why this screen points at the platform-held secrets rather than the
// whole vault.

import { useCallback, useEffect, useState } from "react";
import { ipc } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

export function Capabilities() {
  const [caps, setCaps] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setCaps(extractList(await ipc.listCapabilities()));
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function act(
    id: string,
    action: "approve" | "deny" | "revoke",
  ) {
    setBusyId(id);
    setError(null);
    try {
      if (action === "approve") await ipc.approveCapability(id);
      else if (action === "deny") await ipc.denyCapability(id);
      else await ipc.revokeCapability(id);
      await refresh();
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="flex flex-col h-full">
      <header className="flex items-center gap-2 p-3 border-b border-neutral-200 dark:border-neutral-800">
        <h3 className="text-sm font-medium flex-1">Capability requests</h3>
        <button
          className="btn-secondary"
          onClick={refresh}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh"}
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto p-3 space-y-2 text-sm">
        {error && (
          <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
            {error}
          </div>
        )}

        {caps.length === 0 ? (
          <div className="text-xs text-neutral-500">
            No capability requests. A capability request asks for scoped
            access to a platform-held secret.
          </div>
        ) : (
          caps.map((c) => {
            const id = String(c.id ?? "");
            const status = String(c.status ?? "pending");
            const granted = status === "approved" || status === "granted";
            const busy = busyId === id;
            return (
              <div
                key={id}
                className="border border-neutral-200 dark:border-neutral-800 rounded-md px-3 py-2 text-xs space-y-1"
              >
                <div className="flex items-center gap-2">
                  <span className="font-medium flex-1 truncate">
                    {String(c.agent_name ?? c.name ?? c.agent_id ?? id)}
                  </span>
                  <span className="text-neutral-500">{status}</span>
                </div>
                {c.scopes ? (
                  <div className="text-neutral-500">
                    scopes: {formatScopes(c.scopes)}
                  </div>
                ) : null}
                <div className="flex gap-2 pt-1">
                  {!granted ? (
                    <>
                      <button
                        className="btn-secondary"
                        disabled={busy}
                        onClick={() => act(id, "approve")}
                      >
                        Approve
                      </button>
                      <button
                        className="btn-secondary"
                        disabled={busy}
                        onClick={() => act(id, "deny")}
                      >
                        Deny
                      </button>
                    </>
                  ) : (
                    <button
                      className="btn-secondary"
                      disabled={busy}
                      onClick={() => act(id, "revoke")}
                    >
                      Revoke
                    </button>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

function formatScopes(raw: unknown): string {
  if (Array.isArray(raw)) return raw.map(String).join(", ");
  return String(raw);
}

export function extractList(result: unknown): Array<Record<string, unknown>> {
  if (Array.isArray(result)) {
    return result as Array<Record<string, unknown>>;
  }
  if (result && typeof result === "object") {
    const obj = result as Record<string, unknown>;
    for (const key of ["capabilities", "requests", "items", "data"]) {
      const value = obj[key];
      if (Array.isArray(value)) {
        return value as Array<Record<string, unknown>>;
      }
    }
  }
  return [];
}
