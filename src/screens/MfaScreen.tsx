// The tenant's vault MFA policy and the verification prompt.
//
// A tenant can require a second factor for vault operations. When it does,
// the desktop client has to be able to satisfy it — otherwise the account
// is simply unusable from the desktop app, which is what the previous
// version's behaviour amounted to: it matched the string "mfa" in an error
// message and showed a hint.
//
// The verification code travels in a header, never in a query string,
// because proxies log query strings.

import { useCallback, useEffect, useState } from "react";
import { ipc, type MfaConfig } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

export function MfaScreen() {
  const [config, setConfig] = useState<MfaConfig | null>(null);
  const [code, setCode] = useState("");
  const [loading, setLoading] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setConfig(await ipc.getMfaConfig());
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function verify(e: React.FormEvent) {
    e.preventDefault();
    setVerifying(true);
    setError(null);
    setResult(null);
    try {
      const r = await ipc.verifyMfa(code.trim());
      setResult(
        r.verified
          ? `Verified. The assertion lasts ${r.ttl} seconds.`
          : "The code was rejected.",
      );
      setCode("");
    } catch (err) {
      setError(err instanceof VaultIpcError ? err.message : String(err));
    } finally {
      setVerifying(false);
    }
  }

  return (
    <div className="flex flex-col h-full">
      <header className="flex items-center gap-2 p-3 border-b border-neutral-200 dark:border-neutral-800">
        <h3 className="text-sm font-medium flex-1">Vault MFA</h3>
        <button
          className="btn-secondary"
          onClick={refresh}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh"}
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto p-3 space-y-3 text-sm">
        {error && (
          <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
            {error}
          </div>
        )}

        {config && (
          <div className="grid grid-cols-2 gap-2 text-xs">
            <div className="border border-neutral-200 dark:border-neutral-800 rounded-md px-2 py-1">
              <div className="text-neutral-500">Required</div>
              <div className="font-medium">
                {config.mfa_required ? "yes" : "no"}
              </div>
            </div>
            <div className="border border-neutral-200 dark:border-neutral-800 rounded-md px-2 py-1">
              <div className="text-neutral-500">Method</div>
              <div className="font-medium">{config.mfa_method}</div>
            </div>
          </div>
        )}

        {config && !config.mfa_required && (
          <div className="text-xs text-neutral-500">
            This tenant does not require a second factor for vault
            operations.
          </div>
        )}

        {config?.mfa_required && (
          <form onSubmit={verify} className="space-y-2 max-w-sm">
            <input
              className="input w-full"
              placeholder="Authentication code"
              value={code}
              onChange={(e) => setCode(e.target.value)}
              inputMode="numeric"
              autoComplete="one-time-code"
              required
            />
            <button className="btn-primary" disabled={verifying}>
              {verifying ? "Verifying…" : "Verify code"}
            </button>
          </form>
        )}

        {result && (
          <div className="rounded-md bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-800 px-3 py-2 text-xs text-green-800 dark:text-green-200">
            {result}
          </div>
        )}
      </div>
    </div>
  );
}
