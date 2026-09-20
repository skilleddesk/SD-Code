/**
 * PostCSS pipeline for Tailwind (spec section 4.1: config-driven Tailwind, no styled-components).
 * Tailwind 3 is pinned deliberately: it is the last major version that is configured by
 * tailwind.config.ts, which the locked stack requires.
 */
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
