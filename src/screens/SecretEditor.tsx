// Create / edit modal for a vault secret.
//
// In `create` mode the plaintext is captured here and handed straight to the
// Rust core, which encrypts it under the unlock passphrase before any byte
// leaves the process. In `edit` mode only metadata (name / description /
// scopes) is editable — changing the value is a *rotation*, which is a
// distinct, audited operation surfaced from the detail view.

import { useState } from "react";
import { ipc } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

type Mode = "create" | "edit";

// Mirrors `internal/storage/vault/types.go::SecretType`. `dynamic_credential`
// is intentionally omitted: those are minted by a provider, not typed in.
const SECRET_TYPES = [
  { value: "api_key", label: "API key" },
  { value: "oauth_token", label: "OAuth token" },
  { value: "password", label: "Password" },
  { value: "certificate", label: "Certificate" },
  { value: "federation_key", label: "Federation key" },
] as const;

const NAME_RE = /^[A-Za-z0-9._\-/ ]{1,200}$/;

interface Props {
  mode: Mode;
  /** Present in `edit` mode. */
  id?: string;
  initial?: {
    name?: string;
    description?: string;
    scopes?: string[];
    secret_type?: string;
    namespace?: string;
  };
  onClose: () => void;
  onSaved: () => void;
}

function splitScopes(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

export function SecretEditor({ mode, id, initial, onClose, onSaved }: Props) {
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [secretType, setSecretType] = useState(initial?.secret_type ?? "api_key");
  const [namespace, setNamespace] = useState(initial?.namespace ?? "");
  const [plaintext, setPlaintext] = useState("");
  const [scopes, setScopes] = useState((initial?.scopes ?? []).join(", "));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Ask the platform to hold the value instead of encrypting it here.
  const [serverSide, setServerSide] = useState(false);

  const creating = mode === "create";

  async function save() {
    setError(null);

    const trimmed = name.trim();
    if (!trimmed) {
      setError("Name is required.");
      return;
    }
    if (!NAME_RE.test(trimmed)) {
      setError("Name may only contain letters, numbers, and . _ - / and spaces.");
      return;
    }
    if (creating && plaintext.length === 0) {
      setError("Plaintext value is required.");
      return;
    }

    setBusy(true);
    try {
      if (creating) {
        await ipc.createSecret({
          name: trimmed,
          description: description.trim(),
          secret_type: secretType,
          plaintext,
          namespace: namespace.trim(),
          scopes: splitScopes(scopes),
          server_side: serverSide,
        });
      } else {
        if (!id) throw new Error("missing secret id");
        await ipc.updateSecret({
          id,
          name: trimmed,
          description: description.trim(),
          scopes: splitScopes(scopes),
        });
      }
      // Wipe the plaintext copy we hold as soon as the call returns.
      setPlaintext("");
      onSaved();
    } catch (e) {
      setError(messageFor(e));
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={creating ? "Create secret" : "Edit secret"}
        className="w-full max-w-lg bg-white dark:bg-neutral-950 rounded-lg shadow-2xl flex flex-col"
      >
        <header className="p-4 border-b border-neutral-200 dark:border-neutral-800 flex items-center">
          <h2 className="text-base font-semibold flex-1">
            {creating ? "New secret" : "Edit secret"}
          </h2>
          <button onClick={onClose} className="btn-secondary" disabled={busy}>
            Cancel
          </button>
        </header>

        <div className="p-4 space-y-3 overflow-y-auto">
          {error && (
            <div
              role="alert"
              className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200"
            >
              {error}
            </div>
          )}

          <div>
            <label className="label" htmlFor="se-name">
              Name
            </label>
            <input
              id="se-name"
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. stripe/secret-key"
              autoFocus
            />
          </div>

          {creating && (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="label" htmlFor="se-type">
                  Type
                </label>
                <select
                  id="se-type"
                  className="input"
                  value={secretType}
                  onChange={(e) => setSecretType(e.target.value)}
                >
                  {SECRET_TYPES.map((t) => (
                    <option key={t.value} value={t.value}>
                      {t.label}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="label" htmlFor="se-ns">
                  Namespace
                </label>
                <input
                  id="se-ns"
                  className="input"
                  value={namespace}
                  onChange={(e) => setNamespace(e.target.value)}
                  placeholder="default"
                />
              </div>
            </div>
          )}

          <div>
            <label className="label" htmlFor="se-desc">
              Description
            </label>
            <input
              id="se-desc"
              className="input"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Optional"
            />
          </div>

          <div>
            <label className="label" htmlFor="se-scopes">
              Scopes (comma separated)
            </label>
            <input
              id="se-scopes"
              className="input"
              value={scopes}
              onChange={(e) => setScopes(e.target.value)}
              placeholder="read, inject"
            />
          </div>

          {creating && (
            <div>
              <label className="label" htmlFor="se-value">
                Plaintext value
              </label>
              <textarea
                id="se-value"
                className="input font-mono text-xs min-h-[96px]"
                value={plaintext}
                onChange={(e) => setPlaintext(e.target.value)}
                placeholder="Encrypted locally — the server never sees this."
                spellCheck={false}
              />
              <p className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
                {serverSide
                  ? "The platform encrypts this value with the tenant's data-encryption key. It CAN read the value — and it is the only kind of secret a capability grant can serve."
                  : "Encrypted in the desktop core with your unlock passphrase before it is sent. The orchestrator stores ciphertext only."}
              </p>
            </div>
          )}

          {creating && (
            <label className="flex items-start gap-2 text-xs text-neutral-600 dark:text-neutral-300">
              <input
                type="checkbox"
                checked={serverSide}
                onChange={(e) => setServerSide(e.target.checked)}
                className="mt-0.5"
              />
              <span>
                <strong>Hold this secret in the platform.</strong>{" "}
                Widens the trust boundary — the platform can read the value.
                Required for capability grants and for sharing with other
                tenant members. Requires the admin role.
              </span>
            </label>
          )}

          {!creating && (
            <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
              To change the secret's value, use <strong>Rotate</strong> on the
              secret's detail view — it creates a new audited version.
            </p>
          )}
        </div>

        <footer className="p-4 border-t border-neutral-200 dark:border-neutral-800 flex justify-end gap-2">
          <button onClick={onClose} className="btn-secondary" disabled={busy}>
            Cancel
          </button>
          <button onClick={save} className="btn-primary" disabled={busy}>
            {busy ? "Saving…" : creating ? "Create secret" : "Save changes"}
          </button>
        </footer>
      </div>
    </div>
  );
}

function messageFor(e: unknown): string {
  if (e instanceof VaultIpcError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
