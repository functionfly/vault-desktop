import { useEffect, useRef, useState } from "react";
import { ipc, type RevealResult } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";
import { cancelPendingClear, copySecret } from "../lib/clipboard";
import { SecretAuditDrawer } from "./SecretAuditDrawer";

interface Props {
  id: string;
  onClose: () => void;
  onChanged: () => void;
  onEdit: (id: string) => void;
}

type Tab =
  | "details"
  | "versions"
  | "diff"
  | "rollback"
  | "rotate"
  | "tokens"
  | "audit";

export function SecretDetail({ id, onClose, onChanged, onEdit }: Props) {
  const [tab, setTab] = useState<Tab>("details");
  const [secret, setSecret] = useState<Record<string, unknown> | null>(null);
  const [versions, setVersions] = useState<Record<string, unknown> | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const s = await ipc.getSecret(id);
        if (!cancelled) setSecret(s);
        const v = await ipc.listSecretVersions(id);
        if (!cancelled) setVersions(v);
      } catch (e) {
        if (!cancelled) setError(messageFor(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id]);

  return (
    <div className="fixed inset-0 bg-black/40 flex items-stretch justify-end z-50">
      <div className="w-full max-w-2xl bg-white dark:bg-neutral-950 flex flex-col shadow-2xl">
        <header className="p-4 border-b border-neutral-200 dark:border-neutral-800 flex items-center gap-3">
          <h2 className="text-base font-semibold flex-1 truncate">
            {String(secret?.name ?? "Secret")}
          </h2>
          <button onClick={() => onEdit(id)} className="btn-secondary">
            Edit
          </button>
          <DeleteButton
            id={id}
            name={String(secret?.name ?? "")}
            onDeleted={onClose}
          />
          <button onClick={onClose} className="btn-secondary">
            Close
          </button>
        </header>

        <nav className="flex gap-1 p-2 border-b border-neutral-200 dark:border-neutral-800 text-xs">
          <Pill active={tab === "details"} onClick={() => setTab("details")}>
            Details
          </Pill>
          <Pill active={tab === "versions"} onClick={() => setTab("versions")}>
            Versions
          </Pill>
          <Pill active={tab === "diff"} onClick={() => setTab("diff")}>
            Diff
          </Pill>
          <Pill active={tab === "rotate"} onClick={() => setTab("rotate")}>
            Rotate
          </Pill>
          <Pill active={tab === "rollback"} onClick={() => setTab("rollback")}>
            Rollback
          </Pill>
          <Pill active={tab === "tokens"} onClick={() => setTab("tokens")}>
            Tokens
          </Pill>
          <Pill active={tab === "audit"} onClick={() => setTab("audit")}>
            Audit
          </Pill>
        </nav>

        <div className="flex-1 min-h-0 overflow-y-auto p-4 text-sm space-y-4">
          {error && (
            <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
              {error}
            </div>
          )}
          {tab === "details" && <Details secret={secret} />}
          {tab === "versions" && <Versions versions={versions} />}
          {tab === "diff" && <DiffView secretId={id} />}
          {tab === "rotate" && (
            <Rotate
              secretId={id}
              onDone={() => {
                onChanged();
                onClose();
              }}
            />
          )}
          {tab === "rollback" && (
            <Rollback
              secretId={id}
              versions={versions}
              onDone={() => {
                onChanged();
                onClose();
              }}
            />
          )}
          {tab === "tokens" && (
            <Tokens
              secretId={id}
              onError={setError}
              onChanged={onChanged}
            />
          )}
          {tab === "audit" && <SecretAuditDrawer secretId={id} />}
        </div>
      </div>
    </div>
  );
}

function Pill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full px-3 py-1 ${active ? "bg-brand-600 text-white" : "text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"}`}
    >
      {children}
    </button>
  );
}

function Details({ secret }: { secret: Record<string, unknown> | null }) {
  const [reveal, setReveal] = useState<RevealResult | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // The handle for the pending clipboard wipe, so hiding the value or
  // unmounting can cancel it. Without this, a pending wipe could fire after
  // the user copied something else and destroy their clipboard.
  const clearHandle = useRef<{ cancel: () => void } | null>(null);

  // Cancel any pending wipe when this panel goes away.
  useEffect(() => {
    return () => {
      clearHandle.current?.cancel();
      clearHandle.current = null;
      cancelPendingClear();
    };
  }, []);

  async function doReveal() {
    if (!secret) return;
    setRevealing(true);
    setError(null);
    try {
      const id = String(secret.id);
      const r = await ipc.revealSecret(id);
      setReveal(r);
    } catch (e) {
      setError(messageFor(e));
    } finally {
      setRevealing(false);
    }
  }

  /** Hides the revealed value and cancels any pending clipboard wipe. */
  function hide() {
    clearHandle.current?.cancel();
    clearHandle.current = null;
    cancelPendingClear();
    setReveal(null);
    setCopied(false);
  }

  if (!secret) {
    return <div className="text-neutral-500">Loading…</div>;
  }

  // A platform-held secret has no client-side envelope: the platform
  // encrypted it with the tenant's data-encryption key. Offering a
  // "Reveal" button for it would only produce a confusing failure, so the
  // UI says so plainly instead.
  const platformHeld = secret.zk_mode === false;

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-3 gap-2 text-xs">
        <Field label="ID" value={String(secret.id)} mono />
        <Field label="Type" value={String(secret.secret_type ?? "")} />
        <Field
          label="Namespace"
          value={String(secret.namespace ?? "default")}
        />
        <Field label="Access count" value={String(secret.access_count ?? 0)} />
        <Field label="Created" value={String(secret.created_at ?? "")} />
        <Field label="Updated" value={String(secret.updated_at ?? "")} />
      </div>

      {platformHeld && (
        <div className="rounded-md bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          <strong>Platform-held secret.</strong> The platform encrypts this
          value with the tenant's data-encryption key, so the desktop client
          cannot decrypt it. It can still back a capability grant — that is
          the only kind of secret that can.
        </div>
      )}

      <div>
        <div className="label">Plaintext</div>
        {error && (
          <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200 mb-2">
            {error}
          </div>
        )}
        {reveal ? (
          <div className="space-y-2">
            <pre className="bg-neutral-50 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-md p-3 text-xs font-mono break-all whitespace-pre-wrap">
              {reveal.plaintext}
            </pre>
            <div className="flex gap-2">
              <button
                className="btn-secondary"
                onClick={async () => {
                  try {
                    const handle = await copySecret(reveal.plaintext);
                    clearHandle.current = handle;
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                  } catch {
                    setError("Could not write to the clipboard.");
                  }
                }}
              >
                {copied ? "Copied" : "Copy"}
              </button>
              <button className="btn-secondary" onClick={hide}>
                Hide
              </button>
            </div>
            <div className="text-xs text-neutral-500">
              The clipboard is cleared automatically after 30 seconds.
            </div>
          </div>
        ) : platformHeld ? (
          <div className="text-xs text-neutral-500">
            Not available in the desktop client — this secret is held by the
            platform.
          </div>
        ) : (
          <button
            onClick={doReveal}
            disabled={revealing}
            className="btn-primary"
          >
            {revealing ? "Decrypting…" : "Reveal plaintext"}
          </button>
        )}
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div>
      <div className="label">{label}</div>
      <div className={`text-sm ${mono ? "font-mono" : ""} break-all`}>
        {value || "—"}
      </div>
    </div>
  );
}

function Versions({ versions }: { versions: Record<string, unknown> | null }) {
  if (!versions) return <div className="text-neutral-500">Loading…</div>;
  return (
    <pre className="text-[11px] bg-neutral-50 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-md p-3 whitespace-pre-wrap break-words">
      {JSON.stringify(versions, null, 2)}
    </pre>
  );
}

function DiffView({ secretId }: { secretId: string }) {
  const [from, setFrom] = useState("1");
  const [to, setTo] = useState("2");
  const [data, setData] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  async function go() {
    setError(null);
    try {
      const r = await ipc.diffSecretVersions(
        secretId,
        parseInt(from, 10),
        parseInt(to, 10),
      );
      setData(r);
    } catch (e) {
      setError(messageFor(e));
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex gap-2 items-end">
        <div>
          <div className="label">From version</div>
          <input
            className="input w-24"
            type="number"
            min={1}
            value={from}
            onChange={(e) => setFrom(e.target.value)}
          />
        </div>
        <div>
          <div className="label">To version</div>
          <input
            className="input w-24"
            type="number"
            min={1}
            value={to}
            onChange={(e) => setTo(e.target.value)}
          />
        </div>
        <button onClick={go} className="btn-primary">
          Diff
        </button>
      </div>
      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}
      <pre className="text-[11px] bg-neutral-50 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-md p-3 whitespace-pre-wrap break-words">
        {JSON.stringify(data, null, 2)}
      </pre>
    </div>
  );
}

function Rollback({
  secretId,
  versions,
  onDone,
}: {
  secretId: string;
  versions: Record<string, unknown> | null;
  onDone: () => void;
}) {
  const [target, setTarget] = useState("1");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function go() {
    setBusy(true);
    setError(null);
    try {
      await ipc.rollbackSecret(
        secretId,
        parseInt(target, 10),
        reason || undefined,
      );
      onDone();
    } catch (e) {
      setError(messageFor(e));
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <p className="text-xs text-neutral-500 dark:text-neutral-400">
        Roll back creates a new version whose contents match the chosen
        historical version. The audit log records the action with the reason you
        provide.
      </p>
      <div>
        <div className="label">Target version</div>
        <input
          className="input w-24"
          type="number"
          min={1}
          value={target}
          onChange={(e) => setTarget(e.target.value)}
        />
      </div>
      <div>
        <div className="label">Reason (recorded in audit log)</div>
        <input
          className="input"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          placeholder="e.g. Revert accidental rotation"
        />
      </div>
      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}
      <pre className="text-[10px] text-neutral-500 dark:text-neutral-400 whitespace-pre-wrap break-words">
        {JSON.stringify(versions, null, 2)}
      </pre>
      <button onClick={go} disabled={busy} className="btn-danger">
        {busy ? "Rolling back…" : "Roll back to this version"}
      </button>
    </div>
  );
}

function Rotate({
  secretId,
  onDone,
}: {
  secretId: string;
  onDone: () => void;
}) {
  const [plaintext, setPlaintext] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function go() {
    setError(null);
    if (plaintext.length === 0) {
      setError("New plaintext value is required.");
      return;
    }
    setBusy(true);
    try {
      await ipc.rotateSecret({
        id: secretId,
        plaintext,
        reason: reason.trim(),
      });
      setPlaintext("");
      onDone();
    } catch (e) {
      setError(messageFor(e));
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <p className="text-xs text-neutral-500 dark:text-neutral-400">
        Rotation re-encrypts the secret under a fresh salt and nonce and records
        a new version. The previous value stays readable in version history and
        can be rolled back to.
      </p>
      <div>
        <label className="label" htmlFor="sd-rotate-value">
          New value
        </label>
        <textarea
          id="sd-rotate-value"
          className="input font-mono text-xs min-h-[96px]"
          value={plaintext}
          onChange={(e) => setPlaintext(e.target.value)}
          placeholder="Encrypted locally before it is sent."
          spellCheck={false}
        />
      </div>
      <div>
        <label className="label" htmlFor="sd-rotate-reason">
          Reason (recorded in audit log)
        </label>
        <input
          id="sd-rotate-reason"
          className="input"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          placeholder="e.g. Quarterly key rotation"
        />
      </div>
      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}
      <button onClick={go} disabled={busy} className="btn-primary">
        {busy ? "Rotating…" : "Rotate secret"}
      </button>
    </div>
  );
}

function DeleteButton({
  id,
  name,
  onDeleted,
}: {
  id: string;
  name: string;
  onDeleted: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function doDelete() {
    setBusy(true);
    setError(null);
    try {
      await ipc.deleteSecret(id);
      onDeleted();
    } catch (e) {
      setError(messageFor(e));
      setBusy(false);
      setConfirming(false);
    }
  }

  if (error) {
    return (
      <span className="text-xs text-red-700 dark:text-red-300" role="alert">
        {error}
      </span>
    );
  }

  if (!confirming) {
    return (
      <button onClick={() => setConfirming(true)} className="btn-secondary">
        Delete
      </button>
    );
  }

  return (
    <span className="inline-flex items-center gap-2">
      <span className="text-xs text-neutral-500 dark:text-neutral-400">
        Delete {name ? <strong>{name}</strong> : "this secret"}?
      </span>
      <button onClick={doDelete} disabled={busy} className="btn-danger">
        {busy ? "Deleting…" : "Confirm"}
      </button>
      <button
        onClick={() => setConfirming(false)}
        disabled={busy}
        className="btn-secondary"
      >
        Cancel
      </button>
    </span>
  );
}

// ── Tokens tab ────────────────────────────────────────────────────────────
//
// Per-secret short-lived access tokens. The orchestrator returns the
// plaintext exactly once at mint; thereafter only the SHA-256 hash is
// stored server-side. This tab therefore surfaces the plaintext in a
// copy-to-clipboard modal that auto-closes after the user copies it.
function Tokens({
  secretId,
  onError,
  onChanged,
}: {
  secretId: string;
  onError: (msg: string | null) => void;
  onChanged: () => void;
}) {
  const [tokens, setTokens] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [mintName, setMintName] = useState("");
  const [mintScopes, setMintScopes] = useState("");
  const [mintHours, setMintHours] = useState(24);
  const [minting, setMinting] = useState(false);
  const [minted, setMinted] = useState<{
    token: string;
    name: string;
    expires_at: string;
  } | null>(null);

  async function refresh() {
    setLoading(true);
    try {
      const res = await ipc.listTokens(secretId);
      const arr = Array.isArray(res?.tokens)
        ? (res.tokens as Array<Record<string, unknown>>)
        : [];
      setTokens(arr);
    } catch (e) {
      onError(messageFor(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [secretId]);

  async function mint(e: React.FormEvent) {
    e.preventDefault();
    setMinting(true);
    onError(null);
    try {
      const scopes = mintScopes
        .split(/[,\n]/)
        .map((s) => s.trim())
        .filter(Boolean);
      const res = await ipc.generateToken({
        id: secretId,
        name: mintName.trim() || "ad-hoc",
        scopes,
        expires_in_hours: mintHours,
      });
      // Server returns plaintext exactly once. Capture then refresh the
      // listing so the row appears immediately.
      setMinted({
        token: String(res?.token ?? ""),
        name: String(res?.name ?? mintName),
        expires_at: String(res?.expires_at ?? ""),
      });
      setMintName("");
      setMintScopes("");
      await refresh();
      onChanged();
    } catch (e) {
      onError(messageFor(e));
    } finally {
      setMinting(false);
    }
  }

  async function revoke(tokenId: string) {
    onError(null);
    try {
      await ipc.revokeToken(tokenId);
      await refresh();
      onChanged();
    } catch (e) {
      onError(messageFor(e));
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      onError(`clipboard write failed: ${messageFor(e)}`);
    }
  }

  return (
    <div className="space-y-4">
      <form onSubmit={mint} className="card p-3 space-y-3">
        <h4 className="text-sm font-semibold">Mint a new token</h4>
        <div>
          <label className="label" htmlFor="tok-name">Name</label>
          <input
            id="tok-name"
            className="input"
            placeholder="ci-deployer"
            value={mintName}
            onChange={(e) => setMintName(e.target.value)}
          />
        </div>
        <div>
          <label className="label" htmlFor="tok-scopes">
            Scopes (comma/newline separated)
          </label>
          <input
            id="tok-scopes"
            className="input"
            placeholder="read, vault.read"
            value={mintScopes}
            onChange={(e) => setMintScopes(e.target.value)}
          />
        </div>
        <div>
          <label className="label" htmlFor="tok-hours">Lifetime (hours)</label>
          <input
            id="tok-hours"
            className="input"
            type="number"
            min={1}
            max={8760}
            value={mintHours}
            onChange={(e) => setMintHours(Number(e.target.value) || 24)}
          />
        </div>
        <button
          type="submit"
          disabled={minting}
          className="btn-primary self-start"
        >
          {minting ? "Minting…" : "Mint token"}
        </button>
      </form>

      {minted && (
        <div
          role="dialog"
          aria-label="Token minted"
          className="rounded-md bg-emerald-50 dark:bg-emerald-900/30 border border-emerald-200 dark:border-emerald-800 px-3 py-3 space-y-2"
        >
          <div className="text-sm font-semibold text-emerald-900 dark:text-emerald-100">
            Copy this token now — it will not be shown again.
          </div>
          <div className="text-xs text-emerald-800 dark:text-emerald-200">
            {minted.name}
            {minted.expires_at
              ? ` · expires ${new Date(minted.expires_at).toLocaleString()}`
              : ""}
          </div>
          <pre className="font-mono text-xs break-all bg-neutral-50 dark:bg-neutral-900 p-2 rounded border border-neutral-200 dark:border-neutral-800">
            {minted.token}
          </pre>
          <div className="flex gap-2">
            <button
              onClick={() => copy(minted.token)}
              className="btn-secondary"
            >
              Copy to clipboard
            </button>
            <button
              onClick={() => setMinted(null)}
              className="btn-secondary"
            >
              I have copied it — close
            </button>
          </div>
        </div>
      )}

      <div className="card p-3">
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-sm font-semibold">
            Active tokens ({tokens.length})
          </h4>
          <button
            onClick={refresh}
            disabled={loading}
            className="btn-secondary"
          >
            {loading ? "Refreshing…" : "Refresh"}
          </button>
        </div>
        {tokens.length === 0 ? (
          <div className="text-xs text-neutral-500">No active tokens.</div>
        ) : (
          <ul className="divide-y divide-neutral-100 dark:divide-neutral-800">
            {tokens.map((t) => {
              const id = String(t.id ?? t.token_id ?? "");
              const name = String(t.name ?? "(unnamed)");
              const scopes = Array.isArray(t.scopes)
                ? (t.scopes as string[]).join(", ")
                : "";
              const exp = t.expires_at
                ? new Date(String(t.expires_at)).toLocaleString()
                : "—";
              return (
                <li
                  key={id}
                  className="py-2 flex items-center gap-3"
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{name}</div>
                    <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
                      scopes: <span className="font-mono">{scopes || "—"}</span>
                      <span className="mx-1">·</span>expires <span>{exp}</span>
                    </div>
                  </div>
                  <button
                    onClick={() => revoke(id)}
                    className="btn-secondary"
                  >
                    Revoke
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

function messageFor(e: unknown): string {
  if (e instanceof VaultIpcError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
