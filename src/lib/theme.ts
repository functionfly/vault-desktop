// System theme detection for the vault webview.
//
// WHY THIS EXISTS INSTEAD OF TAILWIND'S `darkMode: "media"` DEFAULT
// ---------------------------------------------------------------
// Tailwind v3 defaults to `@media (prefers-color-scheme: dark)`. Under a
// Tauri webview that media query does NOT reliably track the OS setting:
// on Linux (WebKitGTK) and some Windows webview versions the query stays
// `light` no matter what the desktop theme is, so every `dark:` utility
// silently never activates and the app renders permanently light.
//
// Tauri's own `getCurrentWindow().theme()` DOES ask the windowing system, and
// `onThemeChanged` fires when the user flips their OS theme at runtime. That
// is the authoritative source here, so we read from Tauri and fall back to
// `prefers-color-scheme` only when Tauri is unavailable (plain browser during
// `bun run dev` outside the shell, or the vitest node environment).
//
// We then set BOTH signals on <html>:
//   * `class="dark"`      — drives Tailwind's `dark:` variants (class strategy)
//   * `data-theme="dark"` — drives `color-scheme` in styles.css so scrollbars,
//                           native form controls, and caret colours follow
//
// Keeping both means either could be consumed by a future stylesheet without
// the other silently going stale.
//
// The `@tauri-apps/api/window` import is lazy (same reason as in tauri.ts:
// the module graph must load cleanly under vitest with no Tauri host).

import { isTauri } from "./tauri";

export type Theme = "light" | "dark";

/** User override persisted in localStorage. `system` means "follow the OS". */
export type ThemePreference = Theme | "system";

const STORAGE_KEY = "ff-vault-theme";
const DARK_CLASS = "dark";

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    // Access can throw when cookies/storage are blocked. Theme is a
    // cosmetic concern — degrade to "follow the OS" rather than crash boot.
    return null;
  }
}

export function readPreference(): ThemePreference {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    // Storage can be present but throwing (Safari private mode, blocked
    // third-party context). Theme is cosmetic — never let it break boot.
  }
  return "system";
}

export function writePreference(pref: ThemePreference): void {
  try {
    storage()?.setItem(STORAGE_KEY, pref);
  } catch {
    // See storage() — cosmetic preference, never fatal.
  }
}

/** Tauri reports "Light"/"Dark"; normalise to our lowercase union. */
export function normalizeTheme(raw: unknown): Theme | null {
  if (typeof raw !== "string") return null;
  const v = raw.toLowerCase();
  if (v === "light") return "light";
  if (v === "dark") return "dark";
  return null;
}

/** Browser/dev fallback: read the CSS media query directly. */
export function prefersDarkMediaQuery(): boolean {
  if (typeof globalThis.matchMedia !== "function") return false;
  try {
    return globalThis.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

/**
 * Ask the OS which theme is active.
 *
 * Returns `null` when the answer cannot be determined (no Tauri, IPC error,
 * unrecognised value). Callers fall back to the media query in that case so
 * the app is never stuck with no theme at all.
 */
export async function systemTheme(): Promise<Theme | null> {
  if (!isTauri()) return prefersDarkMediaQuery() ? "dark" : "light";
  try {
    const mod = await import("@tauri-apps/api/window");
    const theme = await mod.getCurrentWindow().theme();
    const normalized = normalizeTheme(theme);
    if (normalized) return normalized;
    return prefersDarkMediaQuery() ? "dark" : "light";
  } catch {
    // IPC unavailable (window tearing down, permission missing on an older
    // build, etc.). The media query is still a reasonable answer.
    return prefersDarkMediaQuery() ? "dark" : "light";
  }
}

/** Resolve the theme to paint, given a preference and the OS answer. */
export function resolveTheme(pref: ThemePreference, system: Theme | null): Theme {
  if (pref === "light" || pref === "dark") return pref;
  return system ?? "light";
}

/**
 * Write the theme onto <html>.
 *
 * Exported (rather than only used internally) so tests can assert against a
 * stub element without a DOM, and so a future manual toggle can re-run the
 * exact same paint path.
 */
export function applyThemeToElement(el: {
  classList: { add(c: string): void; remove(c: string): void; contains(c: string): boolean };
  setAttribute(name: string, value: string): void;
}, theme: Theme): void {
  if (theme === "dark") {
    el.classList.add(DARK_CLASS);
  } else {
    el.classList.remove(DARK_CLASS);
  }
  el.setAttribute("data-theme", theme);
}

export function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  applyThemeToElement(document.documentElement, theme);
}

/**
 * Synchronous first paint, run before React mounts.
 *
 * `initTheme()` awaits the Tauri IPC round-trip, which takes a few ms — long
 * enough to flash a light background at a dark-mode user on every launch.
 * `primeTheme()` paints the best synchronous guess (stored preference, else the
 * media query) immediately; `initTheme()` then corrects it with the
 * authoritative Tauri answer. Both write the same DOM signals, so the
 * correction is a no-op when the guess was already right.
 */
export function primeTheme(): void {
  applyTheme(resolveTheme(readPreference(), prefersDarkMediaQuery() ? "dark" : "light"));
}

/**
 * Resolve the current preference + system theme and paint it.
 * Safe to call repeatedly; idempotent.
 */
export async function initTheme(): Promise<Theme> {
  const theme = resolveTheme(readPreference(), await systemTheme());
  applyTheme(theme);
  return theme;
}

/**
 * Subscribe to OS theme changes and keep the DOM in sync.
 *
 * When the user has pinned an explicit preference we still track the OS value
 * (so un-pinning later is instant) but do not repaint — an explicit choice
 * must not be silently overridden by the system flipping at sunset.
 *
 * Returns an unsubscribe function. Never throws: a listener that cannot be
 * attached just means the app keeps the theme it painted at boot, which is a
 * far better failure than a blank window.
 */
export async function watchSystemTheme(): Promise<() => void> {
  const onChange = (osTheme: Theme) => {
    const pref = readPreference();
    if (pref !== "system") return;
    applyTheme(resolveTheme(pref, osTheme));
  };

  if (!isTauri()) {
    if (typeof globalThis.matchMedia !== "function") return () => {};
    const mql = globalThis.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: { matches: boolean }) =>
      onChange(e.matches ? "dark" : "light");
    // addEventListener on MediaQueryList is the modern API; Safari < 14 only
    // has addListener. Vault targets current WebKitGTK/WKWebView/WebView2, so
    // the modern path is the only one we need — but guard anyway.
    if (typeof mql.addEventListener === "function") {
      mql.addEventListener("change", handler);
      return () => mql.removeEventListener("change", handler);
    }
    return () => {};
  }

  try {
    const mod = await import("@tauri-apps/api/window");
    const unlisten = await mod.getCurrentWindow().onThemeChanged((event) => {
      const normalized = normalizeTheme(event?.payload);
      if (normalized) onChange(normalized);
    });
    return () => {
      try {
        unlisten();
      } catch {
        // Window already destroyed; nothing to clean up.
      }
    };
  } catch {
    return () => {};
  }
}

/**
 * Set an explicit user preference and repaint.
 *
 * "system" clears the override so the OS setting takes over again. The
 * currently-observed system theme is passed in so the paint is synchronous —
 * the caller already has it from initTheme/watchSystemTheme.
 */
export function setPreference(
  pref: ThemePreference,
  system: Theme | null,
): Theme {
  if (pref === "system") {
    try {
      storage()?.removeItem(STORAGE_KEY);
    } catch {
      // See storage().
    }
  } else {
    writePreference(pref);
  }
  const theme = resolveTheme(readPreference(), system);
  applyTheme(theme);
  return theme;
}