// IPC contract test. Verifies that the typed wrappers in src/lib/ipc.ts call
// `@tauri-apps/api/core::invoke` with the right (command, args) shape, and that
// the wrapper's `invoke` indirection parses the `{kind, message}` AppError
// shape into a typed `VaultIpcError`.
//
// Strategy: set `globalThis.__TAURI_INTERNALS__` so `isTauri()` returns true,
// and replace `@tauri-apps/api/core::invoke` with a vitest mock. Every call
// is recorded so we can assert the wire shape without a Tauri runtime.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// --- Global Tauri stub -----------------------------------------------------
//
// `isTauri()` reads `globalThis.__TAURI_INTERNALS__`; set it to a non-undefined
// value so the wrapper reaches `tauriInvoke`. We mock the module separately.
const invokeMock = vi.fn();

beforeEach(() => {
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
    plugins: {},
  };
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  vi.resetModules();
});

// Mock the underlying Tauri invoke so we record every call. vi.mock is hoisted
// so it applies before any import of `./tauri` below.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

// Re-import after each `vi.resetModules()` in tests so the `vi.mock` of
// `@tauri-apps/api/core` is fresh and the `tauri.ts` module re-evaluates
// against it.
async function fresh() {
  const tauri = await import("./tauri");
  const ipc = await import("./ipc");
  return { ipc: ipc.ipc, ...tauri };
}

describe("ipc command dispatch", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("login forwards {baseUrl, email, password} to the login command", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({
      user: { id: "u1", tenant_id: "t1" },
      desktop_capable: true,
    });
    const result = await ipc.login("https://api.example.com", "a@b.test", "pw");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("login", {
      baseUrl: "https://api.example.com",
      email: "a@b.test",
      password: "pw",
    });
    expect(result.user.id).toBe("u1");
    expect(result.desktop_capable).toBe(true);
  });

  it("loginWithApiKey forwards {baseUrl, apiKey}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({
      user: { id: "u2", tenant_id: "t1" },
      desktop_capable: true,
    });
    await ipc.loginWithApiKey("https://api.example.com", "ffk_xxx");
    expect(invokeMock).toHaveBeenCalledWith("login_with_api_key", {
      baseUrl: "https://api.example.com",
      apiKey: "ffk_xxx",
    });
  });

  it("restoreSession sends no args", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ user: undefined, refreshed: false });
    await ipc.restoreSession();
    // Tauri's invoke wrapper normalises undefined → {} before reaching the
    // host, so the mock always sees an args object. The Rust side treats
    // both forms identically (it destructures an Option<HashMap>).
    expect(invokeMock).toHaveBeenCalledWith("restore_session", {});
  });

  it("logout + lockVault + vaultStatus + listNamespaces all pass through unchanged", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce(undefined);
    invokeMock.mockResolvedValueOnce(undefined);
    invokeMock.mockResolvedValueOnce({ authenticated: true, unlocked: false });
    invokeMock.mockResolvedValueOnce({ namespaces: [] });

    await ipc.logout();
    await ipc.lockVault();
    await ipc.vaultStatus();
    await ipc.listNamespaces();

    expect(invokeMock).toHaveBeenNthCalledWith(1, "logout", {});
    expect(invokeMock).toHaveBeenNthCalledWith(2, "lock_vault", {});
    expect(invokeMock).toHaveBeenNthCalledWith(3, "vault_status", {});
    expect(invokeMock).toHaveBeenNthCalledWith(4, "list_namespaces", {});
  });

  it("unlockVault forwards passphrase", async () => {
    const { ipc } = await fresh();
    await ipc.unlockVault("correct horse battery staple");
    expect(invokeMock).toHaveBeenCalledWith("unlock_vault", {
      passphrase: "correct horse battery staple",
    });
  });

  it("listSecrets wraps the query in {query}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ secrets: [], total: 0 });
    await ipc.listSecrets({ search: "db", namespace: "default", limit: 25 });
    expect(invokeMock).toHaveBeenCalledWith("list_secrets", {
      query: { search: "db", namespace: "default", limit: 25 },
    });
  });

  it("getSecret + listSecretVersions both pass {id}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValue({});
    await ipc.getSecret("s1");
    await ipc.listSecretVersions("s1");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_secret", { id: "s1" });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "list_secret_versions", {
      id: "s1",
    });
  });

  it("revealSecret passes {id} and unwraps {secret_id, plaintext}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ secret_id: "s1", plaintext: "hunter2" });
    const r = await ipc.revealSecret("s1");
    expect(invokeMock).toHaveBeenCalledWith("reveal_secret", { id: "s1" });
    expect(r.plaintext).toBe("hunter2");
    expect(r.secret_id).toBe("s1");
  });

  it("diffSecretVersions passes {id, from, to}", async () => {
    const { ipc } = await fresh();
    await ipc.diffSecretVersions("s1", 3, 4);
    expect(invokeMock).toHaveBeenCalledWith("diff_secret_versions", {
      id: "s1",
      from: 3,
      to: 4,
    });
  });

  it('rollbackSecret defaults reason to "" when omitted', async () => {
    const { ipc } = await fresh();
    await ipc.rollbackSecret("s1", 4);
    expect(invokeMock).toHaveBeenLastCalledWith("rollback_secret", {
      id: "s1",
      targetVersion: 4,
      reason: "",
    });
    await ipc.rollbackSecret("s1", 5, "audit");
    expect(invokeMock).toHaveBeenLastCalledWith("rollback_secret", {
      id: "s1",
      targetVersion: 5,
      reason: "audit",
    });
  });

  it("createSecret wraps the input in {input}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ id: "s1" });
    await ipc.createSecret({
      name: "stripe/key",
      secret_type: "api_key",
      plaintext: "sk_live_x",
      namespace: "default",
      scopes: ["read"],
    });
    expect(invokeMock).toHaveBeenCalledWith("create_secret", {
      input: {
        name: "stripe/key",
        secret_type: "api_key",
        plaintext: "sk_live_x",
        namespace: "default",
        scopes: ["read"],
      },
    });
  });

  it("updateSecret wraps the input in {input} and omits untouched fields", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ id: "s1" });
    await ipc.updateSecret({ id: "s1", name: "renamed" });
    expect(invokeMock).toHaveBeenCalledWith("update_secret", {
      input: { id: "s1", name: "renamed" },
    });
  });

  it("rotateSecret wraps the input in {input}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ id: "s1" });
    await ipc.rotateSecret({ id: "s1", plaintext: "new", reason: "rotation" });
    expect(invokeMock).toHaveBeenCalledWith("rotate_secret", {
      input: { id: "s1", plaintext: "new", reason: "rotation" },
    });
  });

  it("deleteSecret passes {id}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce(undefined);
    await ipc.deleteSecret("s1");
    expect(invokeMock).toHaveBeenCalledWith("delete_secret", { id: "s1" });
  });

  it("listAudit passes (secretId, limit) verbatim including undefined", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ entries: [] });
    await ipc.listAudit();
    expect(invokeMock).toHaveBeenLastCalledWith("list_audit", {
      secretId: undefined,
      limit: undefined,
    });
    invokeMock.mockResolvedValueOnce({ entries: [] });
    await ipc.listAudit("s1", 50);
    expect(invokeMock).toHaveBeenLastCalledWith("list_audit", {
      secretId: "s1",
      limit: 50,
    });
  });

  it("listAuditByAction sends {action, limit}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ entries: [] });
    await ipc.listAuditByAction("secret.read", 25);
    expect(invokeMock).toHaveBeenLastCalledWith("list_audit_by_action", {
      action: "secret.read",
      limit: 25,
    });
  });

  it("listAuditByActor sends {actorType, actorId, limit}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ entries: [] });
    await ipc.listAuditByActor("user", "u1", 10);
    expect(invokeMock).toHaveBeenLastCalledWith("list_audit_by_actor", {
      actorType: "user",
      actorId: "u1",
      limit: 10,
    });
  });

  it("createNamespace sends {name}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ id: "ns1" });
    await ipc.createNamespace("prod-keys");
    expect(invokeMock).toHaveBeenLastCalledWith("create_namespace", {
      name: "prod-keys",
    });
  });

  it("deleteNamespace sends {id}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce(undefined);
    await ipc.deleteNamespace("ns1");
    expect(invokeMock).toHaveBeenLastCalledWith("delete_namespace", {
      id: "ns1",
    });
  });

  it("listTokens sends {id}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ tokens: [] });
    await ipc.listTokens("s1");
    expect(invokeMock).toHaveBeenLastCalledWith("list_tokens", { id: "s1" });
  });

  it("generateToken sends {input: {...}} with required fields", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ token: "ffk_xxx", token_id: "t1" });
    await ipc.generateToken({
      id: "s1",
      name: "ci-deployer",
      scopes: ["read"],
      expires_in_hours: 24,
    });
    expect(invokeMock).toHaveBeenLastCalledWith("generate_token", {
      input: {
        id: "s1",
        name: "ci-deployer",
        scopes: ["read"],
        expires_in_hours: 24,
      },
    });
  });

  it("revokeToken sends {tokenId}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce(undefined);
    await ipc.revokeToken("t1");
    expect(invokeMock).toHaveBeenLastCalledWith("revoke_token", {
      tokenId: "t1",
    });
  });

  it("bulkDeleteSecrets sends {input: {secret_ids, dry_run}}", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ previews: {} });
    await ipc.bulkDeleteSecrets({ secret_ids: ["s1", "s2"], dry_run: true });
    expect(invokeMock).toHaveBeenLastCalledWith("bulk_delete_secrets", {
      input: { secret_ids: ["s1", "s2"], dry_run: true },
    });
  });

  it("checkUpdate sends {channel} for the stable channel", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ result: "up_to_date" });
    await ipc.checkUpdate("stable");
    expect(invokeMock).toHaveBeenLastCalledWith("vault_check_update", {
      channel: "stable",
    });
  });

  it("checkUpdate sends {channel} for the beta channel", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({
      result: "available",
      version: "0.2.0",
      notes: "shipped audit filter",
      pub_date: "2026-10-01T00:00:00Z",
    });
    await ipc.checkUpdate("beta");
    expect(invokeMock).toHaveBeenLastCalledWith("vault_check_update", {
      channel: "beta",
    });
  });

  it("applyUpdate sends {channel} and returns the installed-version envelope", async () => {
    const { ipc } = await fresh();
    invokeMock.mockResolvedValueOnce({ installed: { version: "0.2.0" } });
    const r = await ipc.applyUpdate("stable");
    expect(invokeMock).toHaveBeenLastCalledWith("vault_apply_update", {
      channel: "stable",
    });
    expect(r.installed.version).toBe("0.2.0");
  });
});

describe("invoke error normalisation", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("parses {kind, message} from the Rust AppError envelope into VaultIpcError", async () => {
    const { invoke, VaultIpcError } = await fresh();
    invokeMock.mockRejectedValueOnce(
      new Error('{"kind":"VaultLocked","message":"vault is locked"}'),
    );
    await expect(invoke("list_secrets", { query: {} })).rejects.toBeInstanceOf(
      VaultIpcError,
    );
    try {
      await invoke("list_secrets", { query: {} });
    } catch (e) {
      const err = e as { kind: string; message: string };
      expect(err.kind).toBe("VaultLocked");
      expect(err.message).toBe("vault is locked");
    }
  });

  it("falls back to a wrapped Error for non-{kind,message} rejections", async () => {
    const { invoke } = await fresh();
    invokeMock.mockRejectedValueOnce(new Error("connection refused"));
    try {
      await invoke("list_secrets", { query: {} });
      throw new Error("expected invoke to throw");
    } catch (e) {
      expect((e as Error).message).toMatch(/Tauri IPC error/);
      expect((e as Error).message).toContain("connection refused");
    }
  });
});
