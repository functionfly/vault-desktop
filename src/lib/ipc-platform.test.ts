// IPC contract tests for the API surface added in v0.2.0: the audit hash
// chain, capability grants, tenant MFA, dynamic credentials, sharing,
// federation, import/export and idle auto-lock.
//
// Same strategy as `ipc.test.ts`: stub the Tauri host, mock
// `@tauri-apps/api/core::invoke`, and assert the exact (command, args)
// shape on the wire.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

async function fresh() {
  const ipc = await import("./ipc");
  return ipc.ipc;
}

describe("audit hash chain", () => {
  it("getAuditChain calls get_audit_chain with no arguments", async () => {
    const ipc = await fresh();
    await ipc.getAuditChain();
    expect(invokeMock).toHaveBeenCalledWith("get_audit_chain", {});
  });

  it("verifyAuditChain passes the window through", async () => {
    const ipc = await fresh();
    await ipc.verifyAuditChain(500);
    expect(invokeMock).toHaveBeenCalledWith("verify_audit_chain", {
      window: 500,
    });
  });

  it("verifyAuditChain omits the window when none is given", async () => {
    const ipc = await fresh();
    await ipc.verifyAuditChain();
    expect(invokeMock).toHaveBeenCalledWith("verify_audit_chain", {
      window: undefined,
    });
  });

  it("surfaces the intact flag from the verify result", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      intact: false,
      break: { id: "e1", segment: 0, position: 4, reason: "hash mismatch" },
    });
    const result = await ipc.verifyAuditChain(10);
    // The server answers 200 for a broken chain, so the client must read
    // `intact` rather than relying on the absence of an error.
    expect(result.intact).toBe(false);
    expect(result.break?.reason).toBe("hash mismatch");
  });
});

describe("capability grants", () => {
  it("listCapabilities calls list_capabilities", async () => {
    const ipc = await fresh();
    await ipc.listCapabilities();
    expect(invokeMock).toHaveBeenCalledWith("list_capabilities", {});
  });

  it("approveCapability sends the id", async () => {
    const ipc = await fresh();
    await ipc.approveCapability("cap-1");
    expect(invokeMock).toHaveBeenCalledWith("approve_capability", {
      id: "cap-1",
    });
  });

  it("denyCapability sends the id", async () => {
    const ipc = await fresh();
    await ipc.denyCapability("cap-2");
    expect(invokeMock).toHaveBeenCalledWith("deny_capability", {
      id: "cap-2",
    });
  });

  it("revokeCapability sends the id", async () => {
    const ipc = await fresh();
    await ipc.revokeCapability("cap-3");
    expect(invokeMock).toHaveBeenCalledWith("revoke_capability", {
      id: "cap-3",
    });
  });

  it("getSecretPresence sends the secret id", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({ zk_mode: false, present: true });
    const result = await ipc.getSecretPresence("s1");
    expect(invokeMock).toHaveBeenCalledWith("get_secret_presence", {
      id: "s1",
    });
    expect(result.zk_mode).toBe(false);
  });
});

describe("tenant vault MFA", () => {
  it("getMfaConfig calls get_mfa_config", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      mfa_required: true,
      mfa_method: "totp",
    });
    const result = await ipc.getMfaConfig();
    expect(invokeMock).toHaveBeenCalledWith("get_mfa_config", {});
    expect(result.mfa_required).toBe(true);
    expect(result.mfa_method).toBe("totp");
  });

  it("verifyMfa sends the code", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({ verified: true, expires_at: "x", ttl: 900 });
    const result = await ipc.verifyMfa("123456");
    expect(invokeMock).toHaveBeenCalledWith("verify_mfa", { code: "123456" });
    expect(result.verified).toBe(true);
  });
});

describe("dynamic credentials", () => {
  it("createDynamicTarget wraps the input", async () => {
    const ipc = await fresh();
    await ipc.createDynamicTarget({
      name: "prod-db",
      db_type: "postgres",
      host: "db.internal",
      port: 5432,
      database_name: "app",
      admin_username: "root",
      admin_password: "hunter2",
    });
    expect(invokeMock).toHaveBeenCalledWith("create_dynamic_target", {
      input: {
        name: "prod-db",
        db_type: "postgres",
        host: "db.internal",
        port: 5432,
        database_name: "app",
        admin_username: "root",
        admin_password: "hunter2",
      },
    });
  });

  it("listDynamicTargets calls list_dynamic_targets", async () => {
    const ipc = await fresh();
    await ipc.listDynamicTargets();
    expect(invokeMock).toHaveBeenCalledWith("list_dynamic_targets", {});
  });

  it("deleteDynamicTarget sends the id", async () => {
    const ipc = await fresh();
    await ipc.deleteDynamicTarget("t1");
    expect(invokeMock).toHaveBeenCalledWith("delete_dynamic_target", {
      id: "t1",
    });
  });

  it("testDynamicTarget sends the id", async () => {
    const ipc = await fresh();
    await ipc.testDynamicTarget("t1");
    expect(invokeMock).toHaveBeenCalledWith("test_dynamic_target", {
      id: "t1",
    });
  });

  it("createDynamicCredential wraps the input", async () => {
    const ipc = await fresh();
    await ipc.createDynamicCredential({
      target_id: "t1",
      name: "reporting",
    });
    expect(invokeMock).toHaveBeenCalledWith("create_dynamic_credential", {
      input: { target_id: "t1", name: "reporting" },
    });
  });

  it("listDynamicCredentials calls list_dynamic_credentials", async () => {
    const ipc = await fresh();
    await ipc.listDynamicCredentials();
    expect(invokeMock).toHaveBeenCalledWith("list_dynamic_credentials", {});
  });

  it("generateDynamicCredential sends the id and ttl", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      lease_id: "l1",
      username: "app",
      password: "pw",
      host: "db",
      port: 5432,
      database: "app",
      expires_at: "2030-01-01T00:00:00Z",
    });
    const result = await ipc.generateDynamicCredential("c1", 3600);
    expect(invokeMock).toHaveBeenCalledWith("generate_dynamic_credential", {
      id: "c1",
      ttlSeconds: 3600,
    });
    expect(result.lease_id).toBe("l1");
    expect(result.password).toBe("pw");
  });

  it("revokeDynamicCredential sends the id", async () => {
    const ipc = await fresh();
    await ipc.revokeDynamicCredential("c1");
    expect(invokeMock).toHaveBeenCalledWith("revoke_dynamic_credential", {
      id: "c1",
    });
  });

  it("renewLease sends both ids and the ttl", async () => {
    const ipc = await fresh();
    await ipc.renewLease("c1", "l1", 600);
    expect(invokeMock).toHaveBeenCalledWith("renew_lease", {
      credentialId: "c1",
      leaseId: "l1",
      ttlSeconds: 600,
    });
  });

  it("revokeLease sends both ids", async () => {
    const ipc = await fresh();
    await ipc.revokeLease("c1", "l1");
    expect(invokeMock).toHaveBeenCalledWith("revoke_lease", {
      credentialId: "c1",
      leaseId: "l1",
    });
  });
});

describe("sharing and federation", () => {
  it("shareSecret wraps the input", async () => {
    const ipc = await fresh();
    await ipc.shareSecret({
      secret_id: "s1",
      user_id: "u2",
      scopes: ["read"],
    });
    expect(invokeMock).toHaveBeenCalledWith("share_secret", {
      input: { secret_id: "s1", user_id: "u2", scopes: ["read"] },
    });
  });

  it("listSharedSecrets calls list_shared_secrets", async () => {
    const ipc = await fresh();
    await ipc.listSharedSecrets();
    expect(invokeMock).toHaveBeenCalledWith("list_shared_secrets", {});
  });

  it("revokeShare sends the share id", async () => {
    const ipc = await fresh();
    await ipc.revokeShare("sh1");
    expect(invokeMock).toHaveBeenCalledWith("revoke_share", {
      shareId: "sh1",
    });
  });

  it("listFederationKeys calls list_federation_keys", async () => {
    const ipc = await fresh();
    await ipc.listFederationKeys();
    expect(invokeMock).toHaveBeenCalledWith("list_federation_keys", {});
  });

  it("createFederationKey wraps the input", async () => {
    const ipc = await fresh();
    await ipc.createFederationKey({ name: "peer" });
    expect(invokeMock).toHaveBeenCalledWith("create_federation_key", {
      input: { name: "peer" },
    });
  });
});

describe("import and export", () => {
  // These commands deliberately take NO path. The Rust core opens the
  // native file dialog and the user picks the location, so a compromised
  // renderer cannot direct the core at an arbitrary file on disk.
  it("exportSecrets sends no path", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({ path: "/tmp/vault.json", count: 12 });
    const result = await ipc.exportSecrets();
    expect(invokeMock).toHaveBeenCalledWith("export_secrets", {});
    expect(result.count).toBe(12);
  });

  it("readSecretExport sends no path", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      format_version: 1,
      exported_at: "2026-11-04T00:00:00Z",
      base_url: "https://api.functionfly.com",
      tenant_id: "t1",
      count: 3,
      secrets: [],
    });
    const result = await ipc.readSecretExport();
    expect(invokeMock).toHaveBeenCalledWith("read_secret_export", {});
    expect(result.format_version).toBe(1);
  });
});

describe("idle auto-lock", () => {
  it("reportActivity calls report_activity", async () => {
    const ipc = await fresh();
    await ipc.reportActivity();
    expect(invokeMock).toHaveBeenCalledWith("report_activity", {});
  });

  it("getIdleSettings calls get_idle_settings", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      timeout_seconds: 300,
      choices: [0, 60, 300, 900, 1800],
    });
    const result = await ipc.getIdleSettings();
    expect(invokeMock).toHaveBeenCalledWith("get_idle_settings", {});
    expect(result.timeout_seconds).toBe(300);
    expect(result.choices).toContain(300);
  });

  it("setIdleSettings sends the timeout", async () => {
    const ipc = await fresh();
    invokeMock.mockResolvedValueOnce({
      timeout_seconds: 60,
      choices: [0, 60, 300, 900, 1800],
    });
    const result = await ipc.setIdleSettings(60);
    expect(invokeMock).toHaveBeenCalledWith("set_idle_settings", {
      timeoutSeconds: 60,
    });
    expect(result.timeout_seconds).toBe(60);
  });
});

describe("platform-held secrets", () => {
  it("createSecret forwards the server_side flag", async () => {
    const ipc = await fresh();
    await ipc.createSecret({
      name: "platform",
      secret_type: "api_key",
      plaintext: "value",
      server_side: true,
    });
    expect(invokeMock).toHaveBeenCalledWith("create_secret", {
      input: {
        name: "platform",
        description: undefined,
        secret_type: "api_key",
        plaintext: "value",
        namespace: undefined,
        scopes: undefined,
        server_side: true,
      },
    });
  });

  it("createSecret omits server_side when it is not set", async () => {
    const ipc = await fresh();
    await ipc.createSecret({
      name: "zk",
      secret_type: "api_key",
      plaintext: "value",
    });
    const args = invokeMock.mock.calls[0][1] as { input: Record<string, unknown> };
    expect(args.input.server_side).toBeUndefined();
  });
});
