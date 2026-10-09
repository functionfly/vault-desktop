/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // "class" (not Tailwind's default "media"): under a Tauri webview the
  // prefers-color-scheme media query does not reliably track the OS theme,
  // so a media-query strategy renders the app permanently light. `src/lib/theme.ts`
  // reads the theme from Tauri (`getCurrentWindow().theme()` +
  // `onThemeChanged`) and sets `class="dark"` on <html> instead.
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        brand: {
          50: "#eef4ff",
          100: "#d8e6ff",
          200: "#b1caff",
          500: "#4f88ff",
          600: "#1f6cf2",
          700: "#1655c5",
        },
      },
    },
  },
  plugins: [],
};
