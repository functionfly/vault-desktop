// Updater screen. Lets the operator (or end user on `beta`) check for a
// newer release on a given channel and trigger a download + signature
// verify + staged install. The Rust core enforces a `VAULT_UPDATER_PUBKEY`
// requirement (fail-closed when unset), so this screen surfaces a typed
// `missing_pubkey` envelope rather than a stack trace when the binary was
// shipped without a signing key.
//
// Two channels (`stable`, `beta`) cover the launch surface; insider-style
// rapid-iteration channels are a deliberate follow-up — see
// `src-tauri/src/updater.rs`.

import { useCallback, useState } from "react";
import { ipc, type CheckUpdateResult, type UpdaterChannel } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

type Status =
  | { kind: "idle" }
  | { kind: "checking"; channel: UpdaterChannel }
  | {
      kind: "available";
      channel: UpdaterChannel;
      version: string;
      notes: string;
      pub_date: string | null;
    }
  | { kind: "up_to_date"; channel: UpdaterChannel }
  | { kind: "installing"; channel: UpdaterChannel }
  | { kind: "installed"; version: string }
  | { kind: "error"; code: string; message: string };

export function UpdaterScreen() {
  const [channel, setChannel] = useState<UpdaterChannel>("stable");
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const check = useCallback(async () => {
    setStatus({ kind: "checking", channel });
    try {
      const r: CheckUpdateResult = await ipc.checkUpdate(channel);
      if (r.result === "up_to_date") {
        setStatus({ kind: "up_to_date", channel });
        return;
      }
      setStatus({
        kind: "available",
        channel,
        version: r.version,
        notes: r.notes,
        pub_date: r.pub_date,
      });
    } catch (e) {
      const code =
        e instanceof VaultIpcError && e.kind === "unknown"
          ? "transport"
          : e instanceof VaultIpcError
            ? e.kind
            : "unknown";
      const message = e instanceof VaultIpcError ? e.message : String(e);
      setStatus({ kind: "error", code, message });
    }
  }, [channel]);

  const apply = useCallback(async () => {
    if (status.kind !== "available") return;
    setStatus({ kind: "installing", channel: status.channel });
    try {
      const r = await ipc.applyUpdate(status.channel);
      setStatus({ kind: "installed", version: r.installed.version });
    } catch (e) {
      const code =
        e instanceof VaultIpcError && e.kind === "unknown"
          ? "transport"
          : e instanceof VaultIpcError
            ? e.kind
            : "unknown";
      const message = e instanceof VaultIpcError ? e.message : String(e);
      setStatus({ kind: "error", code, message });
    }
  }, [status]);

  return (
    <div className="flex flex-col h-full overflow-auto">
      <header className="p-4 border-b border-neutral-200 dark:border-neutral-800">
        <h2 className="text-lg font-medium">Updater</h2>
        <p className="text-sm text-neutral-500">
          Signed auto-update. The verification key is compiled into this build,
          so the Rust core can check for updates with no configuration; a
          missing or invalid key surfaces a typed <code>missing_pubkey</code>{" "}
          error rather than silently downloading an unsigned bundle.
        </p>
      </header>

      <div className="p-4 space-y-4">
        <div className="flex items-center gap-3">
          <label className="text-sm" htmlFor="updater-channel">
            Channel
          </label>
          <select
            id="updater-channel"
            className="input-base"
            value={channel}
            onChange={(e) => {
              setChannel(e.target.value as UpdaterChannel);
              setStatus({ kind: "idle" });
            }}
            disabled={
              status.kind === "checking" || status.kind === "installing"
            }
          >
            <option value="stable">stable</option>
            <option value="beta">beta</option>
          </select>
          <button
            type="button"
            className="btn-primary"
            onClick={check}
            disabled={
              status.kind === "checking" || status.kind === "installing"
            }
          >
            {status.kind === "checking" ? "Checking…" : "Check for updates"}
          </button>
        </div>

        {status.kind === "up_to_date" && (
          <div className="rounded border border-emerald-200 dark:border-emerald-900 bg-emerald-50 dark:bg-emerald-950/40 p-3 text-sm">
            Up to date on the <code>{status.channel}</code> channel.
          </div>
        )}

        {status.kind === "available" && (
          <div className="rounded border border-amber-200 dark:border-amber-900 bg-amber-50 dark:bg-amber-950/40 p-4 space-y-2">
            <div className="flex items-baseline gap-3">
              <span className="font-medium">{status.version}</span>
              <span className="text-xs text-neutral-500">
                {status.pub_date ?? "no release timestamp"}
              </span>
            </div>
            {status.notes ? (
              <pre className="whitespace-pre-wrap text-xs text-neutral-700 dark:text-neutral-300">
                {status.notes}
              </pre>
            ) : (
              <div className="text-xs text-neutral-500">No release notes.</div>
            )}
            <button
              type="button"
              className="btn-primary"
              onClick={apply}
              disabled={status.kind !== "available"}
            >
              Download + verify + stage
            </button>
            <div className="text-xs text-neutral-500">
              Stages the new bundle and schedules install on the next restart.
              The Rust core downloads, verifies the Ed25519 signature against
              the configured pubkey, and aborts the install on any signature
              mismatch.
            </div>
          </div>
        )}

        {status.kind === "installing" && (
          <div className="rounded border border-blue-200 dark:border-blue-900 bg-blue-50 dark:bg-blue-950/40 p-3 text-sm">
            Downloading + verifying + staging the new bundle on the{" "}
            <code>{status.channel}</code> channel…
          </div>
        )}

        {status.kind === "installed" && (
          <div className="rounded border border-emerald-200 dark:border-emerald-900 bg-emerald-50 dark:bg-emerald-950/40 p-4 space-y-2">
            <div className="font-medium">Installed {status.version}.</div>
            <div className="text-sm text-neutral-600 dark:text-neutral-400">
              Restart the application to apply the new bundle. The Rust core has
              already verified the Ed25519 signature.
            </div>
          </div>
        )}

        {status.kind === "error" && (
          <div className="rounded border border-red-200 dark:border-red-900 bg-red-50 dark:bg-red-950/40 p-3 text-sm space-y-1">
            <div>
              <span className="font-mono text-xs">code={status.code}</span>
            </div>
            <div>{status.message}</div>
            <div className="text-xs text-neutral-500">
              Common causes:
              <ul className="list-disc pl-5 mt-1 space-y-0.5">
                <li>
                  <code>missing_pubkey</code> — this build carries no
                  verification key, and no{" "}
                  <code>VAULT_UPDATER_PUBKEY</code> override was set.
                </li>
                <li>
                  <code>invalid_endpoint</code> — the per-channel URL env var is
                  malformed.
                </li>
                <li>
                  <code>unknown_channel</code> — the channel string wasn't one
                  of <code>stable</code> or <code>beta</code>.
                </li>
                <li>
                  <code>builder_error</code> — the tauri-plugin-updater builder
                  rejected the pubkey/endpoint combination.
                </li>
              </ul>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
