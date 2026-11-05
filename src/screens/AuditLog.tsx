import { useCallback, useEffect, useState } from "react";
import { ipc } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";
import { AuditChainPanel } from "./AuditChainPanel";

export function AuditLog() {
  const [data, setData] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [action, setAction] = useState("");
  const [limit, setLimit] = useState(100);
  const [actorType, setActorType] = useState("");
  const [actorId, setActorId] = useState("");
  const [showChain, setShowChain] = useState(false);

  const fetch_ = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Three filter paths, in priority order:
      //   1. an actor type AND an actor id → the per-actor route
      //   2. an action                 → the per-action route
      //   3. neither                   → everything
      //
      // The per-actor route existed in the IPC layer from the start but
      // was never surfaced here; the comment in this file used to call it
      // "a follow-up".
      const trimmedType = actorType.trim();
      const trimmedId = actorId.trim();
      const r =
        trimmedType && trimmedId
          ? await ipc.listAuditByActor(trimmedType, trimmedId, limit)
          : action.trim()
          ? await ipc.listAuditByAction(action.trim(), limit)
          : await ipc.listAudit(undefined, limit);
      setData(r);
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [action, limit, actorType, actorId]);

  useEffect(() => {
    fetch_();
  }, [fetch_]);

  const entries = extractEntries(data);

  return (
    <div className="flex flex-col h-full">
      <header className="flex items-center gap-2 p-3 border-b border-neutral-200 dark:border-neutral-800">
        <input
          className="input flex-1"
          placeholder="Filter by action (e.g. secret.read)"
          value={action}
          onChange={(e) => setAction(e.target.value)}
        />
        <select
          className="input w-28"
          value={actorType}
          onChange={(e) => setActorType(e.target.value)}
        >
          <option value="">Actor type</option>
          {["user", "token", "system", "api_key"].map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <input
          className="input w-40"
          placeholder="Actor ID"
          value={actorId}
          onChange={(e) => setActorId(e.target.value)}
        />
        <input
          className="input w-24"
          type="number"
          min={1}
          max={1000}
          value={limit}
          onChange={(e) =>
            setLimit(Math.max(1, parseInt(e.target.value, 10) || 100))
          }
        />
        <button onClick={fetch_} className="btn-secondary" disabled={loading}>
          {loading ? "Loading…" : "Refresh"}
        </button>
        <button
          onClick={() => setShowChain((v) => !v)}
          className="btn-secondary"
        >
          {showChain ? "Hide chain" : "Chain integrity"}
        </button>
      </header>

      {showChain && (
        <div className="p-3 border-b border-neutral-200 dark:border-neutral-800">
          <AuditChainPanel />
        </div>
      )}

      {error && (
        <div className="m-3 rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}

      <div className="flex-1 min-h-0 overflow-y-auto">
        {entries.length === 0 ? (
          <div className="p-6 text-sm text-neutral-500">No audit entries.</div>
        ) : (
          <table className="w-full text-xs">
            <thead className="sticky top-0 bg-white dark:bg-neutral-950">
              <tr className="text-left text-neutral-500 border-b border-neutral-200 dark:border-neutral-800">
                <th className="px-3 py-2 font-medium">When</th>
                <th className="px-3 py-2 font-medium">Action</th>
                <th className="px-3 py-2 font-medium">Actor</th>
                <th className="px-3 py-2 font-medium">Secret</th>
                <th className="px-3 py-2 font-medium">Status</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e, i) => (
                <tr
                  key={i}
                  className="border-b border-neutral-100 dark:border-neutral-800"
                >
                  <td className="px-3 py-1.5 whitespace-nowrap font-mono">
                    {String(e.created_at ?? "")}
                  </td>
                  <td className="px-3 py-1.5">{String(e.action ?? "")}</td>
                  <td className="px-3 py-1.5 font-mono">
                    {String(e.actor_type ?? "")}/{String(e.actor_id ?? "")}
                  </td>
                  <td className="px-3 py-1.5 font-mono">
                    {e.secret_id ? String(e.secret_id) : "—"}
                  </td>
                  <td className="px-3 py-1.5">
                    {e.success === false ? (
                      <span className="text-red-600">failure</span>
                    ) : (
                      <span className="text-emerald-600">ok</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

interface Entry {
  id: string;
  action?: string;
  actor_id?: string;
  actor_type?: string;
  secret_id?: string;
  success?: boolean;
  created_at?: string;
  [k: string]: unknown;
}

function extractEntries(data: unknown): Entry[] {
  if (!data || typeof data !== "object") return [];
  const obj = data as { entries?: unknown };
  if (Array.isArray(obj.entries)) return obj.entries as Entry[];
  // Some endpoints use a different key; fall back to scanning the object.
  const arr = Object.values(obj).find((v) => Array.isArray(v));
  return (arr as Entry[] | undefined) ?? [];
}
