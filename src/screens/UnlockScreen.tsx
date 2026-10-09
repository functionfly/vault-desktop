import { useState } from "react";
import { ipc, type StoreBackend } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

interface Props {
  userEmail: string | null;
  /** Orchestrator this session is bound to, so the user knows which
   *  environment they are about to unlock secrets for. */
  baseUrl?: string | null;
  /** Where the long-lived token lives. `file_fallback` is worth flagging
   *  because it is weaker than the OS keyring. */
  storeBackend?: StoreBackend | null;
  onUnlocked: () => Promise<void> | void;
  onSignOut: () => Promise<void> | void;
}

export function UnlockScreen({
  userEmail,
  baseUrl,
  storeBackend,
  onUnlocked,
  onSignOut,
}: Props) {
  const [passphrase, setPassphrase] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await ipc.unlockVault(passphrase);
      // Best-effort wipe of the local copy. The Rust side stores the
      // passphrase in a Zeroizing<String> so the buffer is wiped when
      // the IPC payload is consumed; the webview's copy is cleared here
      // so it does not linger if the user keeps the window open.
      setPassphrase("");
      await onUnlocked();
    } catch (e) {
      const msg = e instanceof VaultIpcError
        ? e.message
        : e instanceof Error
        ? e.message
        : String(e);
      setError(msg);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="w-full max-w-sm card p-6">
        <h1 className="text-lg font-semibold mb-1">Unlock the vault</h1>
        <p className="text-xs text-neutral-500 dark:text-neutral-400 mb-1">
          {userEmail ? `Signed in as ${userEmail}.` : "Signed in to the orchestrator."}
          {baseUrl ? (
            <>
              {" "}
              <span className="font-mono text-[11px]">
                ({baseUrl.replace(/^https?:\/\//, "")})
              </span>
            </>
          ) : null}
        </p>
        <p className="text-xs text-neutral-500 dark:text-neutral-400 mb-6">
          Enter your vault passphrase to decrypt secrets locally. It is never
          sent to the orchestrator and is wiped from memory on lock.
        </p>

        {storeBackend === "file_fallback" && (
          <div className="mb-4 rounded-md border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 text-xs text-amber-900 dark:text-amber-200">
            This machine has no OS keyring, so the session token is sealed under
            a device-local key. It is encrypted at rest, but weaker than a real
            keyring.
          </div>
        )}

        <form onSubmit={submit} className="space-y-4">
          <div>
            <label className="label" htmlFor="passphrase">Vault passphrase</label>
            <input
              id="passphrase"
              className="input"
              type="password"
              required
              autoComplete="current-password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
            />
          </div>

          {error && (
            <div
              role="alert"
              className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200"
            >
              {error}
            </div>
          )}

          <div className="flex gap-2">
            <button type="submit" disabled={submitting} className="btn-primary flex-1">
              {submitting ? "Unlocking…" : "Unlock"}
            </button>
            <button type="button" onClick={onSignOut} className="btn-secondary">
              Sign out
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
