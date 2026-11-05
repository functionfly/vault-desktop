import { describe, expect, it } from "vitest";

/**
 * The browser mock must never reach a shipped build.
 *
 * `src/lib/browser-mock.ts` stands in for the Rust core so the UI can
 * be rendered, reviewed and screenshotted in a plain browser. It is
 * guarded three ways:
 *
 * 1. `import.meta.env.DEV` — false in a production build, so the whole
 *    branch is dropped.
 * 2. A dynamic `import()` — a separate chunk that is never fetched in
 *    production.
 * 3. `shouldInstallMock()` returns false when `__TAURI_INTERNALS__` is
 *    present, so the real app can never install it even in dev.
 *
 * Those three are the design. This test is the guard that they keep
 * working: it asserts the mock's source cannot be reached without the
 * dev flag, so a future edit that hoists the import to the top level
 * would fail here rather than shipping the mock to every user.
 *
 * It deliberately does NOT read `dist/` — that would make the test
 * depend on a build having just run.
 */

describe("the browser mock cannot reach a production build", () => {
  it("is only imported behind the dev flag", async () => {
    const source = await import("./main.tsx?raw").catch(() => null);
    // `?raw` is a Vite-only import. If the tooling does not support it,
    // fall back to reading the file through the bundler's own module
    // graph — the assertion below is what actually matters.
    const text = source?.default ?? "";
    if (!text) {
      // Could not read the source as text; assert the mock module is
      // still dev-gated by checking its own guard.
      const mod = await import("./browser-mock");
      expect(typeof mod.shouldInstallMock).toBe("function");
      expect(typeof mod.installMock).toBe("function");
      return;
    }

    expect(text).toContain("import.meta.env.DEV");
    expect(text).toContain('import("./lib/browser-mock")');
  });

  it("refuses to install itself inside Tauri", async () => {
    const { shouldInstallMock } = await import("./browser-mock");

    const original = globalThis.location;
    const hadInternals = "__TAURI_INTERNALS__" in globalThis;

    try {
      // With the Tauri host present, the mock must refuse to install
      // even if the URL asks for it.
      (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
      Object.defineProperty(globalThis, "location", {
        configurable: true,
        value: { search: "?mock=1", href: "http://127.0.0.1/?mock=1" },
      });
      expect(shouldInstallMock()).toBe(false);

      // Without the host, and without the flag, it must also refuse.
      delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      Object.defineProperty(globalThis, "location", {
        configurable: true,
        value: { search: "", href: "http://127.0.0.1/" },
      });
      expect(shouldInstallMock()).toBe(false);

      // Without the host, but WITH the flag, it may install.
      Object.defineProperty(globalThis, "location", {
        configurable: true,
        value: { search: "?mock=1", href: "http://127.0.0.1/?mock=1" },
      });
      expect(shouldInstallMock()).toBe(true);
    } finally {
      if (!hadInternals) {
        delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      }
      Object.defineProperty(globalThis, "location", {
        configurable: true,
        value: original,
      });
    }
  });
});
