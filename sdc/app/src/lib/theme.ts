/**
 * Theme controller (spec section 8.1: dark is the default theme, light is a complete second set).
 *
 * The values themselves live in exactly one place - src/styles/tokens.css - as CSS custom
 * properties: `:root` is dark, `[data-theme="light"]` re-declares the light set. This module is
 * the only writer of that attribute, so no React component has to know which theme is active and
 * no component can disagree with the cascade. Its consumers are the topbar theme toggle
 * (#themeToggle, spec section 7.1) and the Appearance tab (spec section 9.11).
 *
 * Importing has no side effects: nothing is read or written until a function is called.
 */

/** The two themes src/styles/tokens.css defines. */
export type Theme = 'dark' | 'light';

/** Dark is what `:root` declares, so it is also what a missing or unknown attribute means. */
export const DEFAULT_THEME: Theme = 'dark';

/** `prefers-color-scheme` query; a match means the operating system is asking for dark. */
const SYSTEM_DARK_QUERY = '(prefers-color-scheme: dark)';

/**
 * Applies a theme by setting `<html data-theme="...">`, which is what the token file keys its
 * light overrides off. Setting `dark` again is the same as clearing the attribute, because `:root`
 * is dark already; `light` is the only value that has to be written.
 */
export function setTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/**
 * The theme in force right now.
 *
 * @returns `light` when `<html data-theme="light">` is set, otherwise the dark default.
 */
export function getTheme(): Theme {
  return document.documentElement.dataset.theme === 'light' ? 'light' : DEFAULT_THEME;
}

/** What the operating system prefers right now - the first-run default for the Appearance tab. */
export function getSystemTheme(): Theme {
  const query = matchSystemTheme();

  return query && query.matches ? 'dark' : 'light';
}

/**
 * Subscribes to operating-system theme changes.
 *
 * @param callback invoked with the new theme every time the OS preference flips.
 * @returns the unsubscribe function; call it from the effect's cleanup.
 */
export function onSystemChange(callback: (theme: Theme) => void): () => void {
  const query = matchSystemTheme();

  if (!query) {
    return () => {};
  }

  const handleChange = (event: MediaQueryListEvent): void => {
    callback(event.matches ? 'dark' : 'light');
  };

  query.addEventListener('change', handleChange);

  return () => {
    query.removeEventListener('change', handleChange);
  };
}

/** The `prefers-color-scheme` media query list, or null where the platform has no matchMedia. */
function matchSystemTheme(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(SYSTEM_DARK_QUERY)
    : null;
}
