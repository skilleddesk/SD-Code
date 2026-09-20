import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

/**
 * The rule this file exists to keep: **a store selector must return a stable value**.
 *
 * Zustand 5 hands the selector straight to `useSyncExternalStore`, which calls it on every commit and
 * compares the result with `Object.is`. A selector that builds a value - `state.toasts.map(...)`,
 * `(state) => ({ a: state.a })`, `state.list.filter(...)` - answers with something new every time, so
 * React sees the snapshot change, re-renders, sees it change again, hits its fifty-update limit and
 * unmounts the tree. The window is then black, with a console message about update depth that no user
 * ever reads.
 *
 * That is not a theory: it is what shipped in 0.4.1-0.4.3, from one line in `overlays/Toast.tsx`, and
 * the reason `app/scripts/smoke-bundle.mjs` exists. This test is the cheap half of the guard - it
 * reads the sources instead of running the app - so the mistake is caught by `pnpm test` in the
 * second it takes, in the file that made it. The smoke check is the half that catches a mount which
 * goes wrong for any other reason.
 *
 * The fix is always local: select the stable thing (`state.toasts`), then derive from it in a
 * `useMemo` - or, inside the selector, in one step that ends in a primitive.
 *
 * This file lives outside `src/` because it reads the filesystem: the app's tsconfig is the browser's
 * (`lib: DOM`, `types: vite/client`), and `tsconfig.node.json` is where Node types belong.
 */

const here = dirname(fileURLToPath(import.meta.url));
const sourceRoot = join(here, '..', 'src');

/** The hooks that take a selector: the app store and the slices composed over it. */
const STORE_HOOK = /\buse(?:App|Prefs|Toast|Provider|Sessions|Overlay)Store\s*\(/g;

/** What a selector looks like when it *builds* a value instead of reading one. */
const ARRAY_BUILDERS = ['.map(', '.filter(', '.flatMap(', '.slice(', '.concat(', '.sort(', '.reverse(', '.reduce('];

/** Calls that turn such a value into a primitive, which `Object.is` can compare. */
const PRIMITIVES = ['.join(', '.length', '.indexOf(', '.includes(', '.some(', '.every(', '.find(', '.at(', '.toString(', '.toFixed(', '.replace('];

/** The rule in one function: does this selector's *result* stay stable? */
export function unstable(expression: string): string | null {
  if (/=>\s*\(\{/.test(expression)) {
    return 'an object literal';
  }

  if (/=>\s*\[/.test(expression)) {
    return 'an array literal';
  }

  for (const builder of ARRAY_BUILDERS) {
    const at = expression.lastIndexOf(builder);

    if (at < 0) {
      continue;
    }

    /* Everything after the last builder: `state.toasts.map(..).join('|')` ends in a string, while
       `state.toasts.map(..)` ends in a fresh array. */
    const tail = expression.slice(at);

    if (!PRIMITIVES.some((primitive) => tail.includes(primitive))) {
      return builder;
    }
  }

  return null;
}

/** Every non-test TypeScript source under `src`. */
function sources(directory: string): string[] {
  return readdirSync(directory).flatMap((entry: string): string[] => {
    const path = join(directory, entry);

    if (statSync(path).isDirectory()) {
      return sources(path);
    }

    return path.endsWith('.ts') || path.endsWith('.tsx') ? [path] : [];
  });
}

/** The text of a call's argument list, from its opening parenthesis to the matching one. */
function argumentOf(text: string, openIndex: number): string {
  let depth = 0;

  for (let index = openIndex; index < text.length; index += 1) {
    const character = text[index];

    if (character === '(') {
      depth += 1;
    } else if (character === ')') {
      depth -= 1;

      if (depth === 0) {
        return text.slice(openIndex + 1, index);
      }
    }
  }

  return text.slice(openIndex + 1);
}

/** Every selector that would answer with a new value, as `file:line: hook(...)`. */
export function unstableSelectors(text: string, file: string): string[] {
  const found: string[] = [];

  STORE_HOOK.lastIndex = 0;

  for (const match of text.matchAll(STORE_HOOK)) {
    const argument = argumentOf(text, match.index + match[0].length - 1);
    const reason = unstable(argument);

    if (reason) {
      const line = text.slice(0, match.index).split('\n').length;

      found.push(`${file}:${line}: ${match[0]}${argument.slice(0, 60)}... (${reason})`);
    }
  }

  return found;
}

describe('store selectors', () => {
  it('never build a new value', () => {
    const offenders = sources(sourceRoot)
      .flatMap((path) => unstableSelectors(readFileSync(path, 'utf8'), path.slice(sourceRoot.length + 1)))
      .filter((line) => !line.includes('.test.'));

    expect(offenders, `selectors must return stable values:\n${offenders.join('\n')}`).toEqual([]);
  });

  it('catches the line that shipped 0.4.1-0.4.3', () => {
    /* The bug, as it was: a fresh array on every call. */
    const buggy = 'const holdFors = useAppStore((state) => state.toasts.map((toast) => `${toast.id}:${toast.holdMs}`));';

    expect(unstableSelectors(buggy, 'overlays/Toast.tsx')).toHaveLength(1);

    /* The fix, as it is: the same map, joined into one primitive. */
    const fixed = 'const holdKey = useAppStore((state) => state.toasts.map((toast) => `${toast.id}`).join("|"));';

    expect(unstableSelectors(fixed, 'overlays/Toast.tsx')).toHaveLength(0);

    /* And the everyday selectors, which read a field rather than build one. */
    expect(unstableSelectors('const toasts = useToastStore((state) => state.toasts);', 'overlays/Toast.tsx')).toHaveLength(0);
    expect(unstableSelectors('const n = useAppStore((state) => state.hosts.length);', 'panels/Sidebar.tsx')).toHaveLength(0);
    expect(unstableSelectors('const open = useProviderStore((state) => state.providers.find((p) => p.id === id));', 'x.tsx')).toHaveLength(0);

    /* The other shapes that would loop: an object literal, a filter, a spread. */
    expect(unstableSelectors('const s = useAppStore((state) => ({ hosts: state.hosts }));', 'x.tsx')).toHaveLength(1);
    expect(unstableSelectors('const running = useAppStore((state) => state.turns.filter((turn) => turn.live));', 'x.tsx')).toHaveLength(1);
    expect(unstableSelectors('const ids = useAppStore((state) => [...state.ids]);', 'x.tsx')).toHaveLength(1);
  });
});
