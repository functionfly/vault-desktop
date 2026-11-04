export default {
  plugins: {
    // Tailwind v4 moved its PostCSS plugin into a separate package.
    // `tailwindcss` itself is no longer a PostCSS plugin — pointing at it
    // fails the build with:
    //
    //   [postcss] It looks like you're trying to use `tailwindcss`
    //   directly as a PostCSS plugin.
    //
    "@tailwindcss/postcss": {},
    autoprefixer: {},
  },
};
