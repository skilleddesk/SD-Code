import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: { include: ['e2e/**/*.bench.test.ts'], environment: 'node', testTimeout: 240_000, hookTimeout: 240_000, fileParallelism: false },
});
