// The audit trail for a single secret.
//
// The IPC layer already exposed `list_audit_by_actor` and the
// per-secret audit route, but the AuditLog screen never surfaced them —
// the comment in that file called it "a follow-up". This is that
// follow-up: the per-secret view a user lands on from a secret's
// "Audit" tab.
//
// The actor filter is here because "who touched this secret" is the
// question an auditor actually asks, and the endpoint existed.

import { useCallback, useEffect, useState } from "react";
import { ipc } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

interface Props {
  secretId: string;
}

/** The actor types the orchestrator's audit log records. */
const ACTOR_TYPES = ["user", "token", "system", "api_key"] as const;

export function SecretAuditDrawer({ secretId }: Props) {
  const [entries, setEntries] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actorType, setActorType] = useState<string>("");
  const [actorId, setActorId] = useState<string>("");

  const fetchEntries = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // When both actor filters are set, use the dedicated per-actor
      // route. Otherwise ask for everything this secret touched.
      const trimmedType = actorType.trim();
      const trimmedId = actorId.trim();
      const result =
        trimmedType && trimmedId
          ? await ipc.listAuditByActor(trimmedType, trimmedId, 200)
          : await ipc.listAudit(secretId, 200);
      setEntries(extractEntries(result));
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [secretId, actorType, actorId]);

  useEffect(() => {
    fetchEntries();
  }, [fetchEntries]);

  return (
    <div className="space-y-3">
      <div className="flex gap-2 items-center">
        <select
          className="input w-36"
          value={actorType}
          onChange={(e) => setActorType(e.target.value)}
        >
          <option value="">Any actor type</option>
          {ACTOR_TYPES.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <input
          className="input flex-1"
          placeholder="Actor ID (both fields required)"
          value={actorId}
          onChange={(e) => setActorId(e.target.value)}
        />
        <button
          className="btn-secondary"
          onClick={fetchEntries}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh"}
        </button>
      </div>

      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}

      {entries.length === 0 && !loading ? (
        <div className="text-xs text-neutral-500">
          No audit entries for this secret.
        </div>
      ) : (
        <div className="border border-neutral-200 dark:border-neutral-800 rounded-md overflow-hidden">
          <table className="w-full text-xs">
            <thead className="bg-neutral-50 dark:bg-neutral-900 text-left">
              <tr>
                <th className="px-2 py-1 font-medium">When</th>
                <th className="px-2 py-1 font-medium">Action</th>
                <th className="px-2 py-1 font-medium">Actor</th>
                <th className="px-2 py-1 font-medium">Result</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e, i) => (
                <tr
                  key={String(e.id ?? i)}
                  className="border-t border-neutral-200 dark:border-neutral-800"
                >
                  <td className="px-2 py-1 text-neutral-500">
                    {formatWhen(e.created_at)}
                  </td>
                  <td className="px-2 py-1 font-mono">
                    {String(e.action ?? "")}
                  </td>
                  <td className="px-2 py-1">
                    {String(e.actor_type ?? "")}
                    {e.actor_id ? `:${String(e.actor_id).slice(0, 8)}` : ""}
                  </td>
                  <td className="px-2 py-1">
                    {e.success === false ? (
                      <span className="text-red-600 dark:text-red-400">
                        failed
                      </span>
                    ) : (
                      "ok"
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/**
 * Pulls the entry list out of whichever envelope the orchestrator used.
 *
 * The audit endpoints have returned both a bare array and an object with an
 * `entries` key over time, so both are accepted.
 */
export function extractEntries(result: unknown): Array<Record<string, unknown>> {
  if (Array.isArray(result)) {
    return result as Array<Record<string, unknown>>;
  }
  if (result && typeof result === "object") {
    const obj = result as Record<string, unknown>;
    for (const key of ["entries", "audit_logs", "logs", "items"]) {
      const value = obj[key];
      if (Array.isArray(value)) {
        return value as Array<Record<string, unknown>>;
      }
    }
  }
  return [];
}

function formatWhen(raw: unknown): string {
  if (!raw) return "—";
  const d = new Date(String(raw));
  if (Number.isNaN(d.getTime())) return String(raw);
  return d.toLocaleString();
}
