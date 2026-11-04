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
