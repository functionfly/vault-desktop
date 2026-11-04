/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // "class" (not Tailwind's default "media"): under a Tauri webview the
  // prefers-color-scheme media query does not reliably track the OS theme,
  // so a media-query strategy renders the app permanently light. `src/lib/theme.ts`
  // reads the theme from Tauri (`getCurrentWindow().theme()` +
  // `onThemeChanged`) and sets `class="dark"` on <html> instead.
  //
  // In Tailwind v4 this strategy is declared in CSS, not here — see the
  // `@custom-variant dark` directive in `src/styles.css`. The key is left
  // out deliberately: v4 ignores it, and keeping it would suggest the
  // behaviour is configured in this file when it is not.
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
