import { useState, useEffect, useMemo, useCallback } from "react";
import { ipc, type ListSecretsResult, type VaultStatus } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";
import { SecretDetail } from "./SecretDetail";
import { SecretEditor } from "./SecretEditor";
import { AuditLog } from "./AuditLog";
import { UpdaterScreen } from "./UpdaterScreen";

type Tab = "secrets" | "audit" | "namespaces" | "updater";

interface Props {
  status: VaultStatus;
  onLock: () => Promise<void> | void;
  onSignOut: () => Promise<void> | void;
}

export function SecretsScreen({ status, onLock, onSignOut }: Props) {
  const [tab, setTab] = useState<Tab>("secrets");
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [namespace, setNamespace] = useState("");
  const [data, setData] = useState<ListSecretsResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<string | null>(null);

  // Debounce the search field so we don't fire a request on every keystroke.
  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search), 250);
    return () => clearTimeout(t);
  }, [search]);

  const fetch = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await ipc.listSecrets({
        search: debouncedSearch || undefined,
        namespace: namespace || undefined,
        limit: 100,
      });
      setData(res);
    } catch (e) {
      const msg = e instanceof VaultIpcError
        ? e.message
        : e instanceof Error
        ? e.message
        : String(e);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [debouncedSearch, namespace]);

  useEffect(() => {
    if (tab === "secrets") fetch();
  }, [tab, fetch]);

  const secrets = useMemo(() => data?.secrets ?? [], [data?.secrets]);
  const user = status.user;

  // ── bulk select + bulk delete ──────────────────────────────────────────
  // Selection is scoped to the current view; flipping tabs clears it.
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkError, setBulkError] = useState<string | null>(null);

  // Whenever the visible list changes (filter or refresh), drop anything
  // that is no longer visible — avoids stale selections across refreshes.
  useEffect(() => {
    setSelectedIds((prev) => {
      if (prev.size === 0) return prev;
      const visible = new Set(secrets.map((s) => String(s.id ?? "")));
      const next = new Set<string>();
      for (const id of prev) {
        if (visible.has(id)) next.add(id);
      }
      return next;
    });
  }, [secrets]);

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  function clearSelected() {
    setSelectedIds(new Set());
  }

  function selectAllVisible() {
    setSelectedIds(new Set(secrets.map((s) => String(s.id ?? ""))));
  }

  async function doBulkDelete() {
    if (selectedIds.size === 0) return;
    setBulkBusy(true);
    setBulkError(null);
    try {
      // Two-step: dry-run first so the confirmation modal can surface any
      // blocking dependencies. If the orchestrator reports zero blockers we
      // then fire the real delete.
      const ids = Array.from(selectedIds);
      const preview = await ipc.bulkDeleteSecrets({
        secret_ids: ids,
        dry_run: true,
      });
      const previewObj = (preview ?? {}) as {
        blockers?: Record<string, unknown>;
      };
      const blockerCount = Object.values(previewObj.blockers ?? {}).filter(
        (b) => Array.isArray(b) && (b as unknown[]).length > 0,
      ).length;
      const ok = window.confirm(
        blockerCount === 0
          ? `Delete ${ids.length} secret${ids.length === 1 ? "" : "s"}? This cannot be undone.`
          : `${ids.length} selected · ${blockerCount} have blocking dependencies (see audit log). Delete anyway?`,
      );
      if (!ok) {
        setBulkBusy(false);
        return;
      }
      await ipc.bulkDeleteSecrets({ secret_ids: ids, dry_run: false });
      clearSelected();
      await fetch();
    } catch (e) {
      const msg = e instanceof VaultIpcError
        ? e.message
        : e instanceof Error
        ? e.message
        : String(e);
      setBulkError(msg);
    } finally {
      setBulkBusy(false);
    }
  }

  return (
    <div className="flex h-full">
      {/* Sidebar */}
      <aside className="w-56 flex-shrink-0 border-r border-neutral-200 dark:border-neutral-800 flex flex-col">
        <div className="p-4 border-b border-neutral-200 dark:border-neutral-800">
          <div className="text-sm font-semibold">FunctionFly Vault</div>
          <div className="text-[11px] text-neutral-500 dark:text-neutral-400 truncate">
            {user?.email ?? user?.username ?? user?.id}
          </div>
          {/* Which orchestrator this session is bound to. Without this the
              user has no on-screen way to tell a production session from a
              staging one. */}
          {status.base_url && (
            <a
              href={status.base_url}
              target="_blank"
              rel="noreferrer"
              title={status.base_url}
              className="mt-1 block text-[10px] font-mono text-neutral-400 hover:text-brand-600 dark:hover:text-brand-400 truncate"
            >
              {shortHost(status.base_url)}
            </a>
          )}
          {status.store_backend === "file_fallback" && (
            <div
              className="mt-1 text-[10px] text-amber-600 dark:text-amber-400"
              title="No OS keyring on this machine; the session token is sealed under a device-local AES-GCM key."
            >
              file-encrypted session
            </div>
          )}
        </div>

        <nav className="flex-1 p-2 space-y-1 text-sm">
          <NavButton active={tab === "secrets"} onClick={() => setTab("secrets")}>
            Secrets
            {data ? <span className="ml-auto text-xs text-neutral-400">{data.total}</span> : null}
          </NavButton>
          <NavButton active={tab === "audit"} onClick={() => setTab("audit")}>
            Audit log
          </NavButton>
          <NavButton active={tab === "namespaces"} onClick={() => setTab("namespaces")}>
            Namespaces
          </NavButton>
          <NavButton active={tab === "updater"} onClick={() => setTab("updater")}>
            Updater
          </NavButton>
        </nav>

        <div className="p-2 border-t border-neutral-200 dark:border-neutral-800 space-y-1">
          <button onClick={() => onLock()} className="btn-secondary w-full">
            Lock vault
          </button>
          <button onClick={() => onSignOut()} className="btn-secondary w-full">
            Sign out
          </button>
        </div>
      </aside>

      {/* Main */}
      <main className="flex-1 min-w-0 flex flex-col">
        {tab === "secrets" && (
          <SecretsList
            search={search}
            setSearch={setSearch}
            namespace={namespace}
            setNamespace={setNamespace}
            loading={loading}
            error={error}
            secrets={secrets}
            selectedIds={selectedIds}
            onToggleSelected={toggleSelected}
            onSelectAll={selectAllVisible}
            onClearSelected={clearSelected}
            bulkBusy={bulkBusy}
            bulkError={bulkError}
            onBulkDelete={doBulkDelete}
            onSelect={(id) => setSelectedId(id)}
            onRefresh={fetch}
            onNew={() => setEditorOpen(true)}
          />
        )}
        {tab === "audit" && <AuditLog />}
        {tab === "namespaces" && <NamespaceList />}
        {tab === "updater" && <UpdaterScreen />}

        {selectedId && (
          <SecretDetail
            id={selectedId}
            onClose={() => setSelectedId(null)}
            onChanged={fetch}
            onEdit={(id) => {
              setSelectedId(null);
              setEditTarget(id);
            }}
          />
        )}

        {editorOpen && (
          <SecretEditor
            mode="create"
            onClose={() => setEditorOpen(false)}
            onSaved={() => {
              setEditorOpen(false);
              fetch();
            }}
          />
        )}

        {editTarget && (
          <SecretEditor
            mode="edit"
            id={editTarget}
            initial={secrets.find((s) => String(s.id) === editTarget) as
              | { name?: string; description?: string; scopes?: string[] }
              | undefined}
            onClose={() => setEditTarget(null)}
            onSaved={() => {
              setEditTarget(null);
              fetch();
            }}
          />
        )}
      </main>
    </div>
  );
}

function NavButton({
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
      className={`flex items-center gap-2 rounded-md w-full px-2 py-1.5 text-left ${
        active
          ? "bg-brand-50 text-brand-700 dark:bg-brand-700/20 dark:text-brand-200"
          : "hover:bg-neutral-100 dark:hover:bg-neutral-800"
      }`}
    >
      {children}
    </button>
  );
}

function SecretsList({
  search,
  setSearch,
  namespace,
  setNamespace,
  loading,
  error,
  secrets,
  selectedIds,
  onToggleSelected,
  onSelectAll,
  onClearSelected,
  bulkBusy,
  bulkError,
  onBulkDelete,
  onSelect,
  onRefresh,
  onNew,
}: {
  search: string;
  setSearch: (s: string) => void;
  namespace: string;
  setNamespace: (s: string) => void;
  loading: boolean;
  error: string | null;
  secrets: Array<Record<string, unknown>>;
  selectedIds: Set<string>;
  onToggleSelected: (id: string) => void;
  onSelectAll: () => void;
  onClearSelected: () => void;
  bulkBusy: boolean;
  bulkError: string | null;
  onBulkDelete: () => Promise<void>;
  onSelect: (id: string) => void;
  onRefresh: () => void;
  onNew: () => void;
}) {
  const allSelected =
    secrets.length > 0 &&
    secrets.every((s) => selectedIds.has(String(s.id ?? "")));
  const someSelected = selectedIds.size > 0;

  return (
    <>
      <header className="flex flex-col gap-2 border-b border-neutral-200 dark:border-neutral-800">
        <div className="flex items-center gap-2 p-3">
          <input
            className="input flex-1"
            placeholder="Search by name or path…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <input
            className="input w-40"
            placeholder="Namespace"
            value={namespace}
            onChange={(e) => setNamespace(e.target.value)}
          />
          <button onClick={onRefresh} className="btn-secondary" disabled={loading}>
            {loading ? "Loading…" : "Refresh"}
          </button>
          <button onClick={onNew} className="btn-primary">
            New secret
          </button>
        </div>

        {someSelected && (
          <div className="flex items-center gap-2 px-3 pb-3">
            <span className="text-xs text-neutral-600 dark:text-neutral-400">
              {selectedIds.size} selected
            </span>
            <button
              onClick={onSelectAll}
              className="btn-secondary text-xs"
              disabled={allSelected}
            >
              Select all visible
            </button>
            <button
              onClick={onClearSelected}
              className="btn-secondary text-xs"
            >
              Clear
            </button>
            <button
              onClick={onBulkDelete}
              disabled={bulkBusy}
              className="btn-danger text-xs ml-auto"
            >
              {bulkBusy ? "Deleting…" : `Delete ${selectedIds.size}`}
            </button>
          </div>
        )}

        {bulkError && (
          <div
            role="alert"
            className="mx-3 mb-3 rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200"
          >
            {bulkError}
          </div>
        )}
      </header>

      {error && (
        <div
          role="alert"
          className="m-3 rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200"
        >
          {error}
        </div>
      )}

      <div className="flex-1 min-h-0 overflow-y-auto">
        {secrets.length === 0 && !loading ? (
          <div className="p-6 text-sm text-neutral-500 dark:text-neutral-400">
            No secrets match the current filter. Use <strong>New secret</strong>{" "}
            above to create one — it is encrypted locally before it leaves the
            desktop.
          </div>
        ) : (
          <ul className="divide-y divide-neutral-100 dark:divide-neutral-800">
            {secrets.map((s) => {
              const id = String(s.id ?? "");
              const name = String(s.name ?? "(unnamed)");
              const type = String(s.secret_type ?? "");
              const ns = String(s.namespace ?? "default");
              const isSelected = selectedIds.has(id);
              return (
                <li
                  key={id}
                  className={`px-3 py-2 flex items-center gap-3 ${
                    isSelected
                      ? "bg-brand-50/40 dark:bg-brand-700/10"
                      : "hover:bg-neutral-50 dark:hover:bg-neutral-900"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggleSelected(id)}
                    onClick={(e) => e.stopPropagation()}
                    aria-label={`Select ${name}`}
                    className="h-4 w-4 flex-shrink-0"
                  />
                  <button
                    type="button"
                    onClick={() => onSelect(id)}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="text-sm font-medium truncate">{name}</div>
                    <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
                      <span className="font-mono">{type || "—"}</span>
                      <span className="mx-1">·</span>
                      <span>{ns}</span>
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </>
  );
}

function NamespaceList() {
  const [data, setData] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      const res = await ipc.listNamespaces();
      setData(res);
      setError(null);
    } catch (e) {
      setError(messageFor(e));
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await ipc.createNamespace(name.trim());
      setName("");
      await refresh();
    } catch (e) {
      setError(messageFor(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: string, label: string) {
    if (
      !window.confirm(
        `Delete namespace "${label}"? Secrets assigned to it will be left without a namespace.`,
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await ipc.deleteNamespace(id);
      await refresh();
    } catch (e) {
      setError(messageFor(e));
    } finally {
      setBusy(false);
    }
  }

  const list1 = (data as { namespaces?: Array<Record<string, unknown>> } | null)
    ?.namespaces;

  return (
    <div className="p-4 space-y-4">
      <h2 className="text-sm font-semibold">Namespaces</h2>

      {error && (
        <div className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          {error}
        </div>
      )}

      <form onSubmit={create} className="card p-3 flex items-end gap-2">
        <div className="flex-1">
          <label className="label" htmlFor="ns-name">New namespace</label>
          <input
            id="ns-name"
            className="input"
            placeholder="e.g. staging, ci-deployers"
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={64}
          />
        </div>
        <button
          type="submit"
          className="btn-primary"
          disabled={busy || name.trim().length === 0}
        >
          {busy ? "Creating…" : "Create"}
        </button>
      </form>

      <ul className="divide-y divide-neutral-100 dark:divide-neutral-800 card">
        {!list1 || list1.length === 0 ? (
          <li className="p-3 text-xs text-neutral-500">
            No namespaces yet. Create one above — secrets can be assigned
            to it on creation.
          </li>
        ) : (
          list1.map((ns) => {
            const id = String(ns.id ?? "");
            const nsName = String(ns.name ?? "(unnamed)");
            return (
              <li
                key={id}
                className="px-3 py-2 flex items-center gap-3"
              >
                <div className="min-w-0 flex-1 text-sm">{nsName}</div>
                <button
                  onClick={() => remove(id, nsName)}
                  className="btn-secondary"
                  disabled={busy}
                >
                  Delete
                </button>
              </li>
            );
          })
        )}
      </ul>
    </div>
  );
}

function messageFor(e: unknown): string {
  if (e instanceof VaultIpcError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** `https://api.functionfly.com` → `api.functionfly.com`. Keeps the sidebar
 *  badge on one line without truncating the part that identifies the
 *  environment. */
function shortHost(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/\/+$/, "");
}
