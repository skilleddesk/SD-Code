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
    reporters: ['default'],
  },
});
