/**
 * The window's ways out: the person's own browser and the system clipboard.
 *
 * WHY THIS EXISTS (0.15.8). Windows' WebView2 opens `window.open` and `target="_blank"` links and lets
 * `navigator.clipboard` read and write; macOS' WKWebView and Linux' WebKitGTK do neither. On a Mac the
 * sign-in dialog's "Open link" did nothing, "Copy link" did nothing and the key field's "Paste" did
 * nothing - with no error, because each failure was silent. Everything here goes through Tauri's
 * native plugins first (the OS opener, the OS clipboard) and falls back to the web APIs only in a plain
 * browser (dev, tests). Never call `window.open` or `navigator.clipboard` directly for these.
 */

/** Opens a URL in the person's own browser - through Tauri's shell in the app, a new tab in dev. */
export async function openOutside(url: string): Promise<boolean> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');

    await open(url);

    return true;
  } catch {
    return window.open(url, '_blank', 'noopener') !== null;
  }
}

/** Puts text on the system clipboard. `false` only when every way failed. */
export async function copyText(text: string): Promise<boolean> {
  try {
    const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');

    await writeText(text);

    return true;
  } catch {
    /* Not in the app, or the plugin refused: the web clipboard, then the old selection copy that every
       engine still honours inside a click. */
  }

  try {
    await navigator.clipboard.writeText(text);

    return true;
  } catch {
    return copyBySelection(text);
  }
}

/** The clipboard's text, or `null` when it cannot be read. */
export async function readText(): Promise<string | null> {
  try {
    const { readText: read } = await import('@tauri-apps/plugin-clipboard-manager');

    return await read();
  } catch {
    /* fall through to the web clipboard */
  }

  try {
    return await navigator.clipboard.readText();
  } catch {
    return null;
  }
}

function copyBySelection(text: string): boolean {
  const area = document.createElement('textarea');

  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();

  try {
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    area.remove();
  }
}

/**
 * Sends every `http(s)` link the window shows to the person's browser, wherever it was drawn.
 *
 * A net under the components: a link rendered with a plain `<a href target="_blank">` works on Windows
 * and silently does nothing on macOS and Linux. Installed once, at start.
 */
export function routeLinksOutside(): void {
  document.addEventListener(
    'click',
    (event) => {
      if (event.defaultPrevented || event.button !== 0) {
        return;
      }

      const target = event.target instanceof Element ? event.target.closest('a[href]') : null;
      const href = target?.getAttribute('href') ?? '';

      if (!/^https?:\/\//i.test(href)) {
        return;
      }

      event.preventDefault();
      void openOutside(href);
    },
    /* Bubble, not capture: a component that already opened the link itself (Markdown) prevented the
       default by the time this runs, and is not opened twice. */
    false,
  );
}

/** A shortcut hint in the platform's own words: `Ctrl K` is `⌘ K` on a Mac, where Ctrl is not the key. */
export function platformHint(hint: string): string {
  const mac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

  return mac ? hint.replace(/\bCtrl\b/g, '⌘').replace(/\bShift\b/g, '⇧') : hint;
}
