// Dynamic credentials: short-lived database credentials the platform mints
// on demand.
//
// This is a different kind of secret from the ones in the vault list. A
// dynamic credential is never typed in and never stored — the platform
// mints it against a database target and hands back a username, a password
// and a lease that expires. The screen exists because that plaintext is
// shown exactly once.
//
// The "minted" panel is deliberately modal-ish and says so: once the panel
// closes, the value is gone and a new one must be minted.

import { useCallback, useEffect, useState } from "react";
import {
  ipc,
  type CreateDynamicTargetInput,
  type DynamicDbType,
  type GeneratedCredential,
} from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";
import { cancelPendingClear, copySecret } from "../lib/clipboard";

export function DynamicCredentials() {
  const [targets, setTargets] = useState<Array<Record<string, unknown>>>([]);
  const [credentials, setCredentials] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [minted, setMinted] = useState<GeneratedCredential | null>(null);
  const [showTargetForm, setShowTargetForm] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [t, c] = await Promise.all([
        ipc.listDynamicTargets(),
        ipc.listDynamicCredentials(),
      ]);
      setTargets(extractList(t, "targets"));
      setCredentials(extractList(c, "credentials"));
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function generate(credentialId: string) {
    setError(null);
    try {
      setMinted(await ipc.generateDynamicCredential(credentialId));
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    }
  }

  return (
    <div className="flex flex-col h-full">
      <header className="flex items-center gap-2 p-3 border-b border-neutral-200 dark:border-neutral-800">
        <h3 className="text-sm font-medium flex-1">Dynamic credentials</h3>
        <button
          className="btn-secondary"
          onClick={() => setShowTargetForm((v) => !v)}
        >
          {showTargetForm ? "Cancel" : "New target"}
        </button>
        <button
          className="btn-secondary"
          onClick={refresh}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh"}
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto p-3 space-y-4 text-sm">
        {error && (
          <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
            {error}
          </div>
        )}

        {showTargetForm && (
          <TargetForm
            onCreated={async () => {
              setShowTargetForm(false);
              await refresh();
            }}
            onError={setError}
          />
        )}

        {minted && (
          <MintedPanel
            credential={minted}
            onClose={() => {
              cancelPendingClear();
              setMinted(null);
            }}
          />
        )}

        <section>
          <h4 className="text-xs font-medium text-neutral-500 mb-1">
            Targets
          </h4>
          {targets.length === 0 ? (
            <div className="text-xs text-neutral-500">
              No dynamic targets yet. A target is a database the platform can
              mint credentials for.
            </div>
          ) : (
            <ul className="space-y-1">
              {targets.map((t) => (
                <li
                  key={String(t.id)}
                  className="border border-neutral-200 dark:border-neutral-800 rounded-md px-3 py-2 text-xs flex items-center gap-2"
                >
                  <span className="font-medium flex-1 truncate">
                    {String(t.name ?? t.id)}
                  </span>
                  <span className="text-neutral-500">
                    {String(t.db_type ?? t.dbType ?? "")}
                  </span>
                  <span className="text-neutral-500">
                    {String(t.host ?? "")}:{String(t.port ?? "")}
                  </span>
                  <span
                    className={
                      t.status === "active"
                        ? "text-emerald-600"
                        : "text-neutral-500"
                    }
                  >
                    {String(t.status ?? "")}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section>
          <h4 className="text-xs font-medium text-neutral-500 mb-1">
            Credentials
          </h4>
          {credentials.length === 0 ? (
            <div className="text-xs text-neutral-500">
              No dynamic credentials yet.
            </div>
          ) : (
            <ul className="space-y-1">
              {credentials.map((c) => (
                <li
                  key={String(c.id)}
                  className="border border-neutral-200 dark:border-neutral-800 rounded-md px-3 py-2 text-xs flex items-center gap-2"
                >
                  <span className="font-medium flex-1 truncate">
                    {String(c.name ?? c.id)}
                  </span>
                  <span className="text-neutral-500">
                    ttl {String(c.ttl_seconds ?? c.ttlSeconds ?? "")}s
                  </span>
                  <span
                    className={
                      c.status === "active"
                        ? "text-emerald-600"
                        : "text-neutral-500"
                    }
                  >
                    {String(c.status ?? "")}
                  </span>
                  <button
                    className="btn-secondary"
                    onClick={() => generate(String(c.id))}
                  >
                    Mint
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function TargetForm({
  onCreated,
  onError,
}: {
  onCreated: () => void;
  onError: (message: string | null) => void;
}) {
  const [name, setName] = useState("");
  const [dbType, setDbType] = useState<DynamicDbType>("postgres");
  const [host, setHost] = useState("");
  const [port, setPort] = useState(5432);
  const [database, setDatabase] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    onError(null);
    setBusy(true);
    try {
      const input: CreateDynamicTargetInput = {
        name: name.trim(),
        db_type: dbType,
        host: host.trim(),
        port,
        database_name: database.trim(),
        admin_username: username,
        admin_password: password,
      };
      await ipc.createDynamicTarget(input);
      setName("");
      setHost("");
      setDatabase("");
      setUsername("");
      setPassword("");
      onCreated();
    } catch (err) {
      onError(err instanceof VaultIpcError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      onSubmit={submit}
      className="border border-neutral-200 dark:border-neutral-800 rounded-md p-3 space-y-2"
    >
      <div className="text-xs font-medium">New dynamic target</div>
      <div className="grid grid-cols-2 gap-2">
        <input
          className="input"
          placeholder="Name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
        />
        <select
          className="input"
          value={dbType}
          onChange={(e) => setDbType(e.target.value as DynamicDbType)}
        >
          <option value="postgres">PostgreSQL</option>
          <option value="mysql">MySQL</option>
        </select>
        <input
          className="input"
          placeholder="Host"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          required
        />
        <input
          className="input"
          type="number"
          placeholder="Port"
          value={port}
          onChange={(e) => setPort(parseInt(e.target.value, 10) || 0)}
          required
        />
        <input
          className="input"
          placeholder="Database"
          value={database}
          onChange={(e) => setDatabase(e.target.value)}
          required
        />
        <input
          className="input"
          placeholder="Admin username"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          required
        />
      </div>
      <input
        className="input w-full"
        type="password"
        placeholder="Admin password (encrypted by the platform)"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        required
      />
      <button className="btn-primary" disabled={busy}>
        {busy ? "Creating…" : "Create target"}
      </button>
    </form>
  );
}

function MintedPanel({
  credential,
  onClose,
}: {
  credential: GeneratedCredential;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);

  const secret = `${credential.username}:${credential.password}`;

  return (
    <div className="rounded-md border border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-900/20 p-3 space-y-2">
      <div className="text-xs font-medium text-amber-900 dark:text-amber-100">
        Credential minted — shown once. Copy it now; the platform does not
        store it and cannot show it again.
      </div>
      <pre className="bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-md p-2 text-xs font-mono break-all whitespace-pre-wrap">
        {secret}
      </pre>
      <div className="text-xs text-neutral-600 dark:text-neutral-300">
        {credential.host}:{credential.port} / {credential.database} — expires{" "}
        {new Date(credential.expires_at).toLocaleString()}
      </div>
      <div className="flex gap-2">
        <button
          className="btn-secondary"
          onClick={async () => {
            try {
              await copySecret(secret);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            } catch {
              /* clipboard unavailable */
            }
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
        <button className="btn-secondary" onClick={onClose}>
          Done
        </button>
      </div>
    </div>
  );
}

/**
 * Pulls a list out of whichever envelope the orchestrator used.
 *
 * The dynamic endpoints return either a bare array or an object with a
 * `targets` / `credentials` key, so both are accepted.
 */
export function extractList(
  result: unknown,
  key: string,
): Array<Record<string, unknown>> {
  if (Array.isArray(result)) {
    return result as Array<Record<string, unknown>>;
  }
  if (result && typeof result === "object") {
    const obj = result as Record<string, unknown>;
    const value = obj[key];
    if (Array.isArray(value)) {
      return value as Array<Record<string, unknown>>;
    }
    for (const alt of ["items", "data", "results"]) {
      const v = obj[alt];
      if (Array.isArray(v)) return v as Array<Record<string, unknown>>;
    }
  }
  return [];
}
