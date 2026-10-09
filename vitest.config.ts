import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    globals: false,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
  // tauri/runtime imports shouldn't be tree-shaken out under test, but we
  // still avoid the `@tauri-apps/api/core` real impl by stubbing the global
  // `__TAURI_INTERNALS__` via vi.mock in individual test files.
});
