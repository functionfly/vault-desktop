// Import and export of the vault's secret METADATA.
//
// The export carries no ciphertext and no plaintext. It is a portable
// inventory — what secrets a tenant has, what they are called, which
// namespace they live in — and nothing more.
//
// The file is written by the Rust core, so the webview never needs a
// filesystem scope.
//
// There is deliberately NO PATH FIELD HERE. The Rust core opens the
// native file dialog and the user picks the location. An earlier version
// took the path as an argument, which let a compromised renderer point
// the core at any file on disk — the missing `fs:*` scope meant nothing
// when the core would do the IO on the renderer's behalf.

import { useState } from "react";
import { ipc, type SecretExportSummary } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

export function ImportExport() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState<string | null>(null);
  const [summary, setSummary] = useState<SecretExportSummary | null>(null);

  // A dismissed file dialog is not a failure. The core reports it as a
  // `cancelled` error, and the screen just stands down.
  function reportError(e: unknown) {
    if (e instanceof VaultIpcError && e.kind === "cancelled") {
      setError(null);
      return;
    }
    setError(e instanceof VaultIpcError ? e.message : String(e));
  }

  async function doExport() {
    setBusy(true);
    setError(null);
    setExported(null);
    try {
      const res = await ipc.exportSecrets();
      setExported(
        `Wrote ${res.count} secret${res.count === 1 ? "" : "s"} to ${res.path}`,
      );
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(false);
    }
  }

  async function doRead() {
    setBusy(true);
    setError(null);
    setSummary(null);
    try {
      setSummary(await ipc.readSecretExport());
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="p-4 space-y-4 text-sm max-w-2xl">
      <div>
        <h2 className="text-sm font-semibold">Import / export</h2>
        <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
          An export holds your secret <strong>metadata</strong> — names,
          types, namespaces and scopes. It never contains a ciphertext or a
          plaintext value, so it is safe to store and to move between
          machines.
        </p>
      </div>

      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}

      <div className="flex gap-2">
        <button
          className="btn-secondary"
          onClick={doExport}
          disabled={busy}
        >
          {busy ? "Working…" : "Export…"}
        </button>
        <button
          className="btn-secondary"
          onClick={doRead}
          disabled={busy}
        >
          {busy ? "Working…" : "Inspect…"}
        </button>
      </div>

      <p className="text-xs text-neutral-500 dark:text-neutral-400">
        Both actions open your operating system&apos;s file picker, and the
        location you choose is the only path this app will write to or read
        from.
      </p>

      {exported && (
        <div className="rounded-md bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-800 px-3 py-2 text-xs text-green-800 dark:text-green-200">
          {exported}
        </div>
      )}

      {summary && (
        <div className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Metric label="Format version" value={String(summary.format_version)} />
            <Metric label="Secrets" value={String(summary.count)} />
            <Metric label="Exported at" value={summary.exported_at || "—"} />
            <Metric label="Orchestrator" value={summary.base_url || "—"} />
          </div>
          {summary.secrets.length > 0 && (
            <div className="border border-neutral-200 dark:border-neutral-800 rounded-md overflow-hidden">
              <table className="w-full text-xs">
                <thead className="bg-neutral-50 dark:bg-neutral-900 text-left">
                  <tr>
                    <th className="px-2 py-1 font-medium">Name</th>
                    <th className="px-2 py-1 font-medium">Type</th>
                    <th className="px-2 py-1 font-medium">Namespace</th>
                  </tr>
                </thead>
                <tbody>
                  {summary.secrets.map((s, i) => (
                    <tr
                      key={String(s.id ?? i)}
                      className="border-t border-neutral-200 dark:border-neutral-800"
                    >
                      <td className="px-2 py-1">{String(s.name ?? "")}</td>
                      <td className="px-2 py-1 font-mono">
                        {String(s.secret_type ?? "")}
                      </td>
                      <td className="px-2 py-1">
                        {String(s.namespace ?? "default")}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="border border-neutral-200 dark:border-neutral-800 rounded-md px-2 py-1">
      <div className="text-neutral-500">{label}</div>
      <div className="font-medium break-all">{value}</div>
    </div>
  );
}
