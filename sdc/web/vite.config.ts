import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react()],
  // The desktop app's design tokens are the single source of colour, type and spacing.
  resolve: { alias: { '@tokens': new URL('../app/src/styles/tokens.css', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1') } },
  build: { outDir: 'dist', sourcemap: false, target: 'es2022', assetsInlineLimit: 0 },
  test: { include: ['test/**/*.test.ts', 'test/**/*.test.tsx'], environment: 'node' },
});
