import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { initTheme, primeTheme, watchSystemTheme } from "./lib/theme";
import "./styles.css";

// Theme bootstrap. `primeTheme()` paints a synchronous guess BEFORE the first
// frame so a dark-mode user never sees a white flash; `initTheme()` then
// reconciles against the authoritative Tauri/OS answer; `watchSystemTheme()`
// keeps it live when the user flips their desktop theme while the app is open.
//
// All three are individually failure-tolerant — a themed app that silently
// stays on the boot theme is far better than an unthemed crash on startup.
primeTheme();
void initTheme();
void watchSystemTheme();

// Browser-only mock of the Tauri IPC surface.
//
// The app is a Tauri application, so opened in a plain browser there is
// no host and every IPC call throws — the page renders nothing. That
// makes the interface impossible to review or screenshot in a browser.
//
// The mock is installed ONLY when the URL carries `?mock=1` and the app
// is NOT running inside Tauri. It is behind a dynamic import, so the
// production bundle contains no mock code path at all, and the real app
// never loads it.
//
// It has to be installed BEFORE the first render: the app asks for the
// session during mount, and an unanswered call leaves the UI on the
// login screen with no way forward. A top-level await is the way to
// sequence that — Vite supports it in the entry chunk.
//
// See `src/lib/browser-mock.ts` for the details and the fictional data.
if (import.meta.env.DEV) {
  const { shouldInstallMock, installMock } = await import("./lib/browser-mock");
  if (shouldInstallMock()) {
    installMock();
  }
}

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

// NOTE: there is deliberately no query-cache provider here. Every screen
// reads through the typed IPC wrappers in `src/lib/ipc.ts`, which already
// own their own loading/error state; a second caching layer on top would
// only ever duplicate it.
ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
