import { defineConfig } from 'vitest/config';

/**
 * The unit-test runner.
 *
 * `environment: 'node'` on purpose: everything under test is a pure function of the event log
 * (the reducer, the derivations, the command registry's matching), and a DOM would only hide that.
 * The one thing that does need a window - the loopback transport's `setTimeout` - is covered by the
 * integration run of the app instead, which is where a transport belongs.
 */
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts', 'tests/**/*.test.ts'],
    /* macOS writes a `._name` sidecar next to a file on a volume with no room for its metadata (an exFAT
       stick, a network share). That sidecar is the file's resource fork, not a test - `._Ops.test.ts`
       parses as binary and takes the whole run down with it, which is exactly what happened the first time
       this suite ran from the external drive. */
    exclude: ['**/node_modules/**', '**/dist/**', '**/._*'],
    reporters: ['default'],
  },
});
