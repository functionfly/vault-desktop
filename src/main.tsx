import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { initTheme, primeTheme, watchSystemTheme } from "./lib/theme";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, refetchOnWindowFocus: false, staleTime: 10_000 },
  },
});

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

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>
);
