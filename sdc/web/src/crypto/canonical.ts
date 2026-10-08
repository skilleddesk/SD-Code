// Canonical JSON (RFC 8785, restricted): the bytes an `action_hash` covers.
//
// The browser signs a hash and the daemon recomputes it from its own copy, so both must turn a value into
// the same bytes. `sdcd/src/anywhere/canonical.rs` is the other half; both are tested against
// `protocol/remote-vectors.json`. Differences from plain `JSON.stringify`: keys are sorted by UTF-16 code
// unit (what `Array.sort` does on strings), there is no whitespace, and numbers must be safe integers.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export function canonical(value: Json): string {
  if (value === null) return 'null';
  if (typeof value === 'boolean') return value ? 'true' : 'false';

  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value)) throw new Error(`${value} is not a safe integer; the canonical form carries integers only`);
    return String(value);
  }

  if (typeof value === 'string') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;

  const keys = Object.keys(value).sort();

  return `{${keys.map((key) => `${JSON.stringify(key)}:${canonical(value[key] as Json)}`).join(',')}}`;
}
