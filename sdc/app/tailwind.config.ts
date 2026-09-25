import type { Config } from 'tailwindcss';

/**
 * Tailwind theme for SDC.
 *
 * Two rules from the master spec shape this file:
 *
 * 1. spec section 4.1 - Tailwind is config-driven; no styled-components, no CSS-in-JS.
 * 2. spec section 8.1 - no component may hardcode a colour.
 *
 * So every colour below is an alias for a CSS custom property, and those properties are declared
 * once in src/styles/tokens.css, which mirrors design/tokens.json. The token namespaces decide the
 * class names, so a class spells both the property and the role - `bg-bg-raised`,
 * `text-text-secondary`, `border-border-subtle`, `text-state-error`, `bg-diff-addBg`. Shape and
 * motion aliases follow spec sections 8.3 and 8.4. `theme.extend` keeps Tailwind's own defaults
 * for everything the design system does not name.
 */
const config: Config = {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  /* The shell is dark by default (`<html data-theme="dark">`); tokens re-declare for light. */
  darkMode: ['selector', '[data-theme="dark"]'],
  theme: {
    extend: {
      colors: {
        /* Surfaces (spec section 8.1) */
        bg: {
          base: 'var(--bg-base)',
          raised: 'var(--bg-raised)',
          overlay: 'var(--bg-overlay)',
          input: 'var(--bg-input)',
          hover: 'var(--bg-hover)',
          active: 'var(--bg-active)',
          /* Glass chrome - overlays, palette, topbar (a token spec section 8.1 lists). */
          glass: 'var(--bg-glass)',
        },

        /* Hairlines: 1px everywhere, 2px only for the focus ring (spec section 8.3) */
        border: {
          subtle: 'var(--border-subtle)',
          default: 'var(--border-default)',
          strong: 'var(--border-strong)',
          focus: 'var(--border-focus)',
        },

        /* Foreground */
        text: {
          primary: 'var(--text-primary)',
          secondary: 'var(--text-secondary)',
          muted: 'var(--text-muted)',
          faint: 'var(--text-faint)',
          /* The white that sits on an accent or purple fill (spec section 8.1). */
          'on-accent': 'var(--text-on-accent)',
        },

        /* Brand + accents, each with its tint */
        accent: {
          DEFAULT: 'var(--accent)',
          hover: 'var(--accent-hover)',
          subtle: 'var(--accent-subtle)',
          glow: 'var(--accent-glow)',
          /* The fill a white label sits on (0.7.10): `bg-accent` is a good *text* colour on the dark surfaces
             and only reaches 2.74:1 behind white text, which the accessibility audit fails. */
          fill: 'var(--accent-fill)',
          'fill-hover': 'var(--accent-fill-hover)',
        },
        purple: {
          DEFAULT: 'var(--purple)',
          subtle: 'var(--purple-subtle)',
        },
        green: {
          DEFAULT: 'var(--green)',
          subtle: 'var(--green-subtle)',
        },
        orange: {
          DEFAULT: 'var(--orange)',
          subtle: 'var(--orange-subtle)',
        },
        red: {
          DEFAULT: 'var(--red)',
          subtle: 'var(--red-subtle)',
          /* Text on a red tint, one step brighter than --red (the error card's title). */
          bright: 'var(--red-bright)',
        },

        /* Host / session state dots and chips (spec section 5.4) */
        state: {
          idle: 'var(--state-idle)',
          running: 'var(--state-running)',
          waiting: 'var(--state-waiting)',
          /* Five components wrote `text-state-warning`, a class that never existed, so their amber fell
             back to the inherited colour. It is the same token as `waiting`. */
          warning: 'var(--state-waiting)',
          success: 'var(--state-success)',
          error: 'var(--state-error)',
        },

        /* Diff rows (spec section 7.4) */
        diff: {
          addBg: 'var(--diff-add-bg)',
          removeBg: 'var(--diff-remove-bg)',
          addText: 'var(--diff-add-text)',
          removeText: 'var(--diff-remove-text)',
        },
      },
      fontFamily: {
        /* Self-hosted through fontsource in src/main.tsx (spec section 4.1 - no CDN). */
        ui: 'Inter, system-ui, -apple-system, sans-serif',
        mono: 'JetBrains Mono, Menlo, Consolas, monospace',
      },
      borderRadius: {
        xs: 'var(--r-xs)',
        sm: 'var(--r-sm)',
        md: 'var(--r-md)',
        lg: 'var(--r-lg)',
        xl: 'var(--r-xl)',
        full: 'var(--r-full)',
      },
      boxShadow: {
        sm: 'var(--shadow-sm)',
        md: 'var(--shadow-md)',
        lg: 'var(--shadow-lg)',
        xl: 'var(--shadow-xl)',
      },
      /* Motion (spec section 8.4): 90 / 150 / 220ms and the three curves. */
      transitionDuration: {
        fast: 'var(--fast)',
        base: 'var(--base)',
        slow: 'var(--slow)',
      },
      transitionTimingFunction: {
        ease: 'var(--ease)',
        'ease-out': 'var(--ease-out)',
        spring: 'var(--spring)',
      },
      /* Shell metrics from spec section 7.2, declared in src/styles/globals.css. */
      width: {
        sidebar: 'var(--sidebar-w)',
        rightpanel: 'var(--rightpanel-w)',
      },
      minWidth: {
        sidebar: 'var(--sidebar-min-w)',
        rightpanel: 'var(--rightpanel-min-w)',
        main: 'var(--main-min-w)',
      },
      height: {
        topbar: 'var(--topbar-h)',
        statusbar: 'var(--statusbar-h)',
      },

      /*
       * Breakpoints (spec section 7.2, the prototype's media queries). Tailwind 3 wants min-width
       * screens, but the prototype - and therefore the spec - is written as `max-width`, so every
       * one of them is added as a named `max-*` variant and the defaults are kept. `max-900:hidden`
       * is the utility form of `@media (max-width: 900px){ ... display:none }`.
       *
       * 1200 and 900 are shell widths: src/layout/Shell.css and src/layout/useShellLayout.ts hold
       * the same two numbers, because one is the CSS and the other is the store sync. The other
       * four are per-component (spec sections 7.1, 7.4, 7.6, 7.15).
       */
      screens: {
        'max-1100': { max: '1100px' },
        'max-900': { max: '900px' },
        'max-700': { max: '700px' },
        'max-600': { max: '600px' },
      },

      /*
       * Motion (spec section 8.4). `spin` and `pulse` are Tailwind's own; the five below are the
       * prototype's. They are keyframes rather than CSS classes so a component writes
       * `animate-pulse-dot` instead of a literal animation, exactly like `animate-spin`.
       */
      keyframes: {
        /* `.host-status.connecting` - the halo that says "talking to the host". */
        pulseRing: {
          '0%, 100%': { boxShadow: '0 0 0 0 var(--accent-glow)' },
          '50%': { boxShadow: '0 0 0 6px transparent' },
        },
        /* `.sdot.running` / `.tab-dot.running` - a session that is producing tokens. */
        pulseDot: {
          '0%, 100%': { boxShadow: '0 0 0 0 var(--accent-glow)' },
          '50%': { boxShadow: '0 0 0 4px transparent' },
        },
        /* The model dropdown, which opens upward from the prompt toolbar (spec section 9.3). */
        dropUp: {
          from: { opacity: '0', transform: 'translateY(8px) scale(.98)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        /* Popovers, which hang off the element that opened them (spec section 9.5). */
        dropDown: {
          from: { opacity: '0', transform: 'translateY(-6px) scale(.98)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        /* Toasts rise into the bottom of the window (spec section 9.14). */
        toastIn: {
          from: { transform: 'translateY(12px) scale(.96)', opacity: '0' },
          to: { transform: 'translateY(0) scale(1)', opacity: '1' },
        },
      },
      animation: {
        'pulse-ring': 'pulseRing 1.5s infinite',
        'pulse-dot': 'pulseDot 2s ease-in-out infinite',
        'drop-up': 'dropUp 180ms var(--spring)',
        'drop-down': 'dropDown 180ms var(--spring)',
        'toast-in': 'toastIn 240ms var(--spring)',
      },
    },
  },
  /* No extra plugins: everything the design system needs is a token alias above. */
  plugins: [],
};

export default config;
