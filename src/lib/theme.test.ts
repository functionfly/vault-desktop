// Theme-engine unit tests.
//
// vitest.config.ts pins `environment: "node"` (no jsdom is installed), so
// these tests exercise the pure logic — normalisation, resolution, and the
// DOM write — against a hand-rolled stub element rather than a real document.
// That is deliberate: the DOM write is the part most likely to regress
// (silently dropping the `dark` class is invisible until someone runs the app
// in dark mode), so it is exactly what needs pinning here.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
    applyThemeToElement,
    normalizeTheme,
    prefersDarkMediaQuery,
    readPreference,
    resolveTheme,
    setPreference,
} from "./theme";

/** The two themes this module paints. */
type ThemeValue = "light" | "dark";

/** Minimal stand-in for document.documentElement. */
function stubElement() {
  const classes = new Set<string>();
  const attrs = new Map<string, string>();
  return {
    classList: {
      add: (c: string) => void classes.add(c),
      remove: (c: string) => void classes.delete(c),
      contains: (c: string) => classes.has(c),
    },
    setAttribute: (name: string, value: string) => void attrs.set(name, value),
    // test-only inspectors
    _classes: classes,
    _attrs: attrs,
  };
}

/** In-memory localStorage stand-in (node env has none). */
function stubStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  const s = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    clear: () => map.clear(),
  };
  (globalThis as { localStorage?: unknown }).localStorage = s;
  return s;
}

beforeEach(() => {
  stubStorage();
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
    plugins: {},
  };
  vi.resetModules();
});

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete (globalThis as { localStorage?: unknown }).localStorage;
  delete (globalThis as { matchMedia?: unknown }).matchMedia;
});

// --- normalizeTheme --------------------------------------------------------

describe("normalizeTheme", () => {
  it("accepts the Title-Cased values Tauri's Rust enum emits", () => {
    expect(normalizeTheme("Light")).toBe("light");
    expect(normalizeTheme("Dark")).toBe("dark");
  });

  it("accepts already-lowercase values", () => {
    expect(normalizeTheme("light")).toBe("light");
    expect(normalizeTheme("dark")).toBe("dark");
  });

  it("is case-insensitive", () => {
    expect(normalizeTheme("DARK")).toBe("dark");
    expect(normalizeTheme("LiGhT")).toBe("light");
  });

  it("returns null for anything unrecognised", () => {
    // Tauri can return null when it cannot determine the theme; we must not
    // coerce that into "light" and silently override a dark-mode user.
    expect(normalizeTheme(null)).toBeNull();
    expect(normalizeTheme(undefined)).toBeNull();
    expect(normalizeTheme("")).toBeNull();
    expect(normalizeTheme("sepia")).toBeNull();
    expect(normalizeTheme(42)).toBeNull();
    expect(normalizeTheme({})).toBeNull();
  });
});

// --- resolveTheme ----------------------------------------------------------

describe("resolveTheme", () => {
  it("honours an explicit light/dark preference over the OS", () => {
    expect(resolveTheme("light", "dark")).toBe("light");
    expect(resolveTheme("dark", "light")).toBe("dark");
  });

  it("follows the OS when the preference is system", () => {
    expect(resolveTheme("system", "dark")).toBe("dark");
    expect(resolveTheme("system", "light")).toBe("light");
  });

  it("falls back to light when the OS answer is unknown and no override", () => {
    expect(resolveTheme("system", null)).toBe("light");
  });

  it("still honours an override when the OS answer is unknown", () => {
    expect(resolveTheme("dark", null)).toBe("dark");
    expect(resolveTheme("light", null)).toBe("light");
  });
});

// --- applyThemeToElement ---------------------------------------------------

describe("applyThemeToElement", () => {
  it("adds the dark class and sets data-theme=dark", () => {
    const el = stubElement();
    applyThemeToElement(el, "dark");
    expect(el.classList.contains("dark")).toBe(true);
    expect(el._attrs.get("data-theme")).toBe("dark");
  });

  it("removes the dark class and sets data-theme=light", () => {
    const el = stubElement();
    applyThemeToElement(el, "dark");
    applyThemeToElement(el, "light");
    expect(el.classList.contains("dark")).toBe(false);
    expect(el._attrs.get("data-theme")).toBe("light");
  });

  it("is idempotent — repeated application converges", () => {
    const el = stubElement();
    applyThemeToElement(el, "dark");
    applyThemeToElement(el, "dark");
    expect(el.classList.contains("dark")).toBe(true);
    expect(el._attrs.get("data-theme")).toBe("dark");
  });

  it("toggling back and forth leaves no stale class", () => {
    const el = stubElement();
    const cycle: ThemeValue[] = ["light", "dark", "light", "dark", "light"];
    for (const t of cycle) applyThemeToElement(el, t);

    // Final write is "light" — the earlier dark writes must not linger.
    expect(el.classList.contains("dark")).toBe(false);
    expect(el._attrs.get("data-theme")).toBe("light");

    applyThemeToElement(el, "dark");
    expect(el.classList.contains("dark")).toBe(true);
    expect(el._attrs.get("data-theme")).toBe("dark");
  });
});

// --- prefersDarkMediaQuery -------------------------------------------------

describe("prefersDarkMediaQuery", () => {
  it("returns false when matchMedia is unavailable", () => {
    delete (globalThis as { matchMedia?: unknown }).matchMedia;
    expect(prefersDarkMediaQuery()).toBe(false);
  });

  it("returns false when matchMedia throws", () => {
    (globalThis as { matchMedia?: unknown }).matchMedia = () => {
      throw new Error("blocked");
    };
    expect(prefersDarkMediaQuery()).toBe(false);
  });

  it("reads .matches from the dark media query", () => {
    const fn = vi.fn().mockReturnValue({ matches: true });
    (globalThis as { matchMedia?: unknown }).matchMedia = fn;
    expect(prefersDarkMediaQuery()).toBe(true);
    expect(fn).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
  });
});

// --- preference persistence ------------------------------------------------

describe("preference persistence", () => {
  it("defaults to system when nothing is stored", () => {
    expect(readPreference()).toBe("system");
  });

  it("round-trips an explicit preference", () => {
    setPreference("dark", "light");
    expect(readPreference()).toBe("dark");
    setPreference("light", "dark");
    expect(readPreference()).toBe("light");
  });

  it("clears the override when set back to system", () => {
    setPreference("dark", "light");
    expect(readPreference()).toBe("dark");
    setPreference("system", "light");
    expect(readPreference()).toBe("system");
  });

  it("ignores a corrupt stored value", () => {
    (globalThis as { localStorage?: Storage }).localStorage!.setItem(
      "ff-vault-theme",
      "hotdog",
    );
    expect(readPreference()).toBe("system");
  });

  it("survives localStorage throwing (private mode / blocked storage)", () => {
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    };
    // A cosmetic preference must never be able to crash boot.
    expect(readPreference()).toBe("system");
    expect(() => setPreference("dark", "light")).not.toThrow();
    expect(() => setPreference("system", "light")).not.toThrow();
  });
});

// --- systemTheme (Tauri path) ---------------------------------------------

describe("systemTheme", () => {
  it("reads the OS theme from Tauri and normalises the casing", async () => {
    const themeMock = vi.fn().mockResolvedValue("Dark");
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({ theme: themeMock }),
    }));
    vi.resetModules();
    const { systemTheme: fresh } = await import("./theme");
    expect(await fresh()).toBe("dark");
    expect(themeMock).toHaveBeenCalled();
  });

  it("falls back to the media query when Tauri returns null", async () => {
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({ theme: vi.fn().mockResolvedValue(null) }),
    }));
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: true });
    vi.resetModules();
    const { systemTheme: fresh } = await import("./theme");
    expect(await fresh()).toBe("dark");
  });

  it("falls back to the media query when the IPC call throws", async () => {
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({
        theme: vi.fn().mockRejectedValue(new Error("no window")),
      }),
    }));
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: false });
    vi.resetModules();
    const { systemTheme: fresh } = await import("./theme");
    expect(await fresh()).toBe("light");
  });

  it("uses the media query directly when not under Tauri", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: true });
    vi.resetModules();
    const { systemTheme: fresh } = await import("./theme");
    expect(await fresh()).toBe("dark");
  });
});

// --- watchSystemTheme (live OS changes) -----------------------------------

describe("watchSystemTheme", () => {
  it("repaints on OS change only when preference is system", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const handlers: Array<(e: { matches: boolean }) => void> = [];
    const removeEventListener = vi.fn();
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({
      matches: false,
      addEventListener: (_: string, h: (e: { matches: boolean }) => void) =>
        handlers.push(h),
      removeEventListener,
    });

    vi.resetModules();
    const { watchSystemTheme: fresh, applyTheme: apply } = await import(
      "./theme"
    );

    const el = stubElement();
    (globalThis as { document?: unknown }).document = { documentElement: el };
    const unlisten = await fresh();

    apply("light");
    handlers[0]({ matches: true });
    expect(el._attrs.get("data-theme")).toBe("dark");
    expect(el.classList.contains("dark")).toBe(true);

    unlisten();
    expect(removeEventListener).toHaveBeenCalled();
    delete (globalThis as { document?: unknown }).document;
  });

  it("does NOT repaint when the user pinned an explicit theme", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const handlers: Array<(e: { matches: boolean }) => void> = [];
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({
      matches: false,
      addEventListener: (_: string, h: (e: { matches: boolean }) => void) =>
        handlers.push(h),
      removeEventListener: () => {},
    });

    vi.resetModules();
    const { setPreference: setPref, watchSystemTheme: fresh } = await import(
      "./theme"
    );
    setPref("light", "light");

    const el = stubElement();
    (globalThis as { document?: unknown }).document = { documentElement: el };
    const { applyTheme: apply } = await import("./theme");
    apply("light");

    await fresh();
    handlers[0]({ matches: true }); // OS flips to dark
    // Explicit "light" must win — the system flipping at sunset is not
    // permission to override the user's choice.
    expect(el._attrs.get("data-theme")).toBe("light");
    expect(el.classList.contains("dark")).toBe(false);

    delete (globalThis as { document?: unknown }).document;
  });

  it("returns a no-op unlisten when the Tauri listener cannot attach", async () => {
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({
        onThemeChanged: () => Promise.reject(new Error("window gone")),
      }),
    }));
    vi.resetModules();
    const { watchSystemTheme: fresh } = await import("./theme");
    const unlisten = await fresh();
    expect(() => unlisten()).not.toThrow();
  });

  it("subscribes via Tauri onThemeChanged under the Tauri runtime", async () => {
    const unlistenMock = vi.fn();
    const handlers: Array<(e: { payload: string }) => void> = [];
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({
        onThemeChanged: (h: (e: { payload: string }) => void) => {
          handlers.push(h);
          return Promise.resolve(unlistenMock);
        },
      }),
    }));
    vi.resetModules();

    const el = stubElement();
    (globalThis as { document?: unknown }).document = { documentElement: el };
    const { applyTheme: apply, watchSystemTheme: fresh } = await import(
      "./theme"
    );
    apply("light");

    const unlisten = await fresh();
    handlers[0]({ payload: "Dark" });
    expect(el._attrs.get("data-theme")).toBe("dark");
    expect(el.classList.contains("dark")).toBe(true);

    unlisten();
    expect(unlistenMock).toHaveBeenCalled();
    delete (globalThis as { document?: unknown }).document;
  });
});

// --- primeTheme (pre-paint, no flash) -------------------------------------

describe("primeTheme", () => {
  it("paints synchronously from the media query before React mounts", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: true });
    const el = stubElement();
    (globalThis as { document?: unknown }).document = { documentElement: el };

    vi.resetModules();
    const { primeTheme } = await import("./theme");
    // Synchronous — no await. This is the whole point: it runs before the
    // first paint so a dark-mode user never sees a white flash.
    primeTheme();
    expect(el.classList.contains("dark")).toBe(true);
    delete (globalThis as { document?: unknown }).document;
  });

  it("respects a stored explicit preference over the media query", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    (globalThis as { matchMedia?: unknown }).matchMedia = () => ({ matches: true });
    stubStorage({ "ff-vault-theme": "light" });
    const el = stubElement();
    (globalThis as { document?: unknown }).document = { documentElement: el };

    vi.resetModules();
    const { primeTheme } = await import("./theme");
    primeTheme();
    expect(el._attrs.get("data-theme")).toBe("light");
    expect(el.classList.contains("dark")).toBe(false);
    delete (globalThis as { document?: unknown }).document;
  });
});