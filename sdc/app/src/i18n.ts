/**
 * The window's languages (0.12, Global UI).
 *
 * `strings.ts` stays the one place every word lives, in English. A locale is a *pack*: the same shape,
 * any part of it, in another language - a key the pack does not have falls back to English, so a
 * half-translated surface is still a working one. Functions (`(n) => …`) are translated as functions,
 * which is how plurals and word order stay the translator's decision rather than the code's.
 *
 * The choice is read once, at start: switching language reloads the window, because some surfaces take
 * their labels when their module loads (the right panel's tab list), and a reload is the honest way to
 * make every one of them speak the new language.
 */

export type Locale = 'en' | 'bn' | 'hi' | 'ar' | 'ur' | 'es' | 'pt' | 'fr' | 'id' | 'zh';

export interface LocaleInfo {
  id: Locale;
  /** The language's own name for itself. */
  name: string;
  /** Written right to left: the layout mirrors, code and terminals stay left to right. */
  rtl: boolean;
}

export const LOCALES: readonly LocaleInfo[] = [
  { id: 'en', name: 'English', rtl: false },
  { id: 'bn', name: 'বাংলা', rtl: false },
  { id: 'hi', name: 'हिन्दी', rtl: false },
  { id: 'ar', name: 'العربية', rtl: true },
  { id: 'ur', name: 'اردو', rtl: true },
  { id: 'es', name: 'Español', rtl: false },
  { id: 'pt', name: 'Português', rtl: false },
  { id: 'fr', name: 'Français', rtl: false },
  { id: 'id', name: 'Bahasa Indonesia', rtl: false },
  { id: 'zh', name: '中文', rtl: false },
];

const KEY = 'sdc.ui.language';

/** The chosen language: stored, else the system's when SDC speaks it, else English. */
export function currentLocale(): Locale {
  try {
    const stored = globalThis.localStorage?.getItem(KEY);

    if (stored !== null && stored !== undefined && LOCALES.some((locale) => locale.id === stored)) {
      return stored as Locale;
    }
  } catch {
    /* No storage: the system's language, below. */
  }

  const system = (globalThis.navigator?.language ?? 'en').split('-')[0] ?? 'en';

  return (LOCALES.find((locale) => locale.id === system)?.id ?? 'en') as Locale;
}

export function localeInfo(locale: Locale = currentLocale()): LocaleInfo {
  return LOCALES.find((entry) => entry.id === locale) ?? LOCALES[0]!;
}

/** Stores the choice and reloads, so every surface speaks it. */
export function chooseLocale(locale: Locale): void {
  try {
    globalThis.localStorage?.setItem(KEY, locale);
  } catch {
    /* The choice still applies after the reload through the system language. */
  }

  globalThis.location?.reload();
}

/** Sets `<html lang dir>`; code, terminals and diffs carry `dir="ltr"` themselves. */
export function applyDocumentLocale(): void {
  const info = localeInfo();

  if (typeof document !== 'undefined') {
    document.documentElement.lang = info.id;
    document.documentElement.dir = info.rtl ? 'rtl' : 'ltr';
  }
}

/** Every string widened (a pack's value need not equal the English literal), every level optional. */
type Widen<T> = T extends string
  ? string
  : T extends (...args: infer A) => infer R
    ? (...args: A) => Widen<R>
    : T extends readonly (infer U)[]
      ? readonly Widen<U>[]
      : T extends object
        ? { [K in keyof T]: Widen<T[K]> }
        : T;

export type Pack<T> = T extends string
  ? string
  : T extends (...args: never[]) => unknown
    ? Widen<T>
    : T extends readonly unknown[]
      ? Widen<T>
      : T extends object
        ? { [K in keyof T]?: Pack<T[K]> }
        : T;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The base with the pack laid over it: objects merge key by key, everything else is replaced. */
export function merge<T>(base: T, pack: unknown): T {
  if (!isPlainObject(base) || !isPlainObject(pack)) {
    return (pack === undefined ? base : pack) as T;
  }

  const out: Record<string, unknown> = { ...base };

  for (const [key, value] of Object.entries(pack)) {
    if (value === undefined) {
      continue;
    }

    out[key] = key in base ? merge((base as Record<string, unknown>)[key], value) : value;
  }

  return out as T;
}
