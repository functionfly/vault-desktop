// The audit hash chain's tamper-evidence status.
//
// A secrets manager that shows "here is your audit log" but cannot show
// "and nobody has tampered with it" is asking the user to take the log's
// integrity on faith. The orchestrator maintains a per-tenant hash chain
// where each entry commits to its own content AND to the previous entry's
// hash, so a rewritten entry breaks every link after it.
//
// The important subtlety, and the reason this panel is worded carefully:
// an UNKEYED chain detects accidental corruption but NOT a deliberate
// rewrite. The server reports `tamper_evident: false` in that case, and
// this panel says exactly that rather than implying the log is proof.

import { useCallback, useEffect, useState } from "react";
import {
  ipc,
  type AuditChainStatus,
  type AuditChainVerifyResult,
} from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

/** How many of the most recent entries a verify re-hashes. */
const DEFAULT_WINDOW = 1000;

export function AuditChainPanel() {
  const [status, setStatus] = useState<AuditChainStatus | null>(null);
  const [verify, setVerify] = useState<AuditChainVerifyResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setStatus(await ipc.getAuditChain());
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
  }, [fetchStatus]);

  async function runVerify() {
    setVerifying(true);
    setError(null);
    try {
      // The server answers 200 whether or not the chain verified, so the
      // result — not the absence of an error — is the answer.
      setVerify(await ipc.verifyAuditChain(DEFAULT_WINDOW));
    } catch (e) {
      setError(e instanceof VaultIpcError ? e.message : String(e));
    } finally {
      setVerifying(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <button
          className="btn-secondary"
          onClick={fetchStatus}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh status"}
        </button>
        <button
          className="btn-secondary"
          onClick={runVerify}
          disabled={verifying}
        >
          {verifying ? "Verifying…" : "Verify chain"}
        </button>
      </div>

      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}

      {status && (
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Metric label="Entries" value={String(status.total_entries)} />
          <Metric label="Active segment" value={String(status.active_segment)} />
          <Metric label="Posture" value={status.posture} />
          <Metric
            label="Tamper-evident"
            value={status.tamper_evident ? "yes" : "no"}
          />
        </div>
      )}

      {status && !status.tamper_evident && (
        <div className="rounded-md bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          This chain is <strong>not keyed</strong>. It detects accidental
          corruption, but a deliberate rewrite would not be caught. Do not
          treat the log as proof of integrity.
        </div>
      )}

      {verify && (
        <div
          className={`rounded-md border px-3 py-2 text-xs ${
            verify.intact
              ? "bg-green-50 dark:bg-green-900/30 border-green-200 dark:border-green-800 text-green-800 dark:text-green-200"
              : "bg-red-50 dark:bg-red-900/30 border-red-200 dark:border-red-800 text-red-800 dark:text-red-200"
          }`}
        >
          {verify.intact ? (
            <>
              <strong>Chain intact.</strong> {verify.verified} of{" "}
              {verify.total} entries verified
              {verify.legacy > 0
                ? ` (${verify.legacy} predate the chain and are not verifiable)`
                : ""}
              .
            </>
          ) : (
            <>
              <strong>Chain broken.</strong> The first entry that does not
              verify is at segment {verify.break?.segment ?? "?"}, position{" "}
              {verify.break?.position ?? "?"}
              {verify.break?.reason ? `: ${verify.break.reason}` : ""}.
            </>
          )}
          {verify.checked_at && (
            <div className="mt-1 text-neutral-500">
              Checked {new Date(verify.checked_at).toLocaleString()}
            </div>
          )}
        </div>
      )}

      {status && status.segments.length > 0 && (
        <div className="border border-neutral-200 dark:border-neutral-800 rounded-md overflow-hidden">
          <table className="w-full text-xs">
            <thead className="bg-neutral-50 dark:bg-neutral-900 text-left">
              <tr>
                <th className="px-2 py-1 font-medium">Segment</th>
                <th className="px-2 py-1 font-medium">Entries</th>
                <th className="px-2 py-1 font-medium">Anchored</th>
                <th className="px-2 py-1 font-medium">Anchor valid</th>
              </tr>
            </thead>
            <tbody>
              {status.segments.map((seg) => (
                <tr
                  key={seg.segment}
                  className="border-t border-neutral-200 dark:border-neutral-800"
                >
                  <td className="px-2 py-1">{seg.segment}</td>
                  <td className="px-2 py-1">{seg.entries}</td>
                  <td className="px-2 py-1">{seg.anchored ? "yes" : "no"}</td>
                  <td className="px-2 py-1">
                    {seg.anchored ? (seg.anchor_valid ? "yes" : "NO") : "—"}
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

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="border border-neutral-200 dark:border-neutral-800 rounded-md px-2 py-1">
      <div className="text-neutral-500">{label}</div>
      <div className="font-medium">{value}</div>
    </div>
  );
}
