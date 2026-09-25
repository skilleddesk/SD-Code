/** What a person typed, as a URL the frame may load - `http(s)` only, `localhost:3000` accepted. */
export function previewAddress(raw: string): string | null {
  const text = raw.trim();

  if (text === '') {
    return null;
  }

  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `http://${text}`;

  try {
    const url = new URL(withScheme);

    return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : null;
  } catch {
    return null;
  }
}
