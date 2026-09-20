import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';
import tseslint from 'typescript-eslint';

/**
 * ESLint 9 flat config (the shape the Vite React-TS template ships).
 * Only the frontend is linted: Rust is covered by `cargo fmt` / `cargo clippy`.
 */

/* Spec section 8.1: "no component may hardcode a colour - semantic names only". The design-system
   guard further down enforces that in the two component trees that draw UI chrome. */
const HEX_COLOUR_MESSAGE =
  'No raw hex colour (spec section 8.1) - use a semantic token instead, e.g. bg-bg-raised, ' +
  'text-text-secondary, border-border-subtle, state-error. Tokens: app/src/styles/tokens.css.';

export default tseslint.config(
  {
    ignores: ['dist', 'node_modules', 'src-tauri/target', 'src-tauri/gen'],
  },
  {
    files: ['**/*.{ts,tsx}'],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
    },
  },

  /**
   * Design-system guard (spec section 8.1): no raw hex colour inside the two component trees that
   * render UI chrome - src/panels and src/modals. Colour has to come from a token alias, and the
   * three selectors cover the three ways a literal could sneak in:
   *
   *   Literal         '#5B9CFF', 'border-[#282E3B]', style={{ color: '#fff' }}
   *   TemplateElement `bg-[#${shade}]`-style templates
   *   JSXText         text typed straight into JSX
   *
   * The regex is the one the spec asks for: /#[0-9A-Fa-f]{3,8}/.
   */
  {
    files: ['src/panels/**/*.{ts,tsx}', 'src/modals/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-syntax': [
        'error',
        { selector: 'Literal[value=/#[0-9A-Fa-f]{3,8}/]', message: HEX_COLOUR_MESSAGE },
        { selector: 'TemplateElement[value.raw=/#[0-9A-Fa-f]{3,8}/]', message: HEX_COLOUR_MESSAGE },
        { selector: 'JSXText[value=/#[0-9A-Fa-f]{3,8}/]', message: HEX_COLOUR_MESSAGE },
      ],
    },
  },
);
