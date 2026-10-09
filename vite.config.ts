import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Vite dev server runs at 5184 (the URL the Tauri `tauri.conf.json` points
// to). In production Tauri serves the contents of `dist/`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5184,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
  envPrefix: ["VITE_", "TAURI_"],
});
