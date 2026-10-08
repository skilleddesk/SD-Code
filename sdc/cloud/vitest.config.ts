import { cloudflareTest } from '@cloudflare/vitest-plugin';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: './wrangler.jsonc' },
      miniflare: {
        // Throwaway values for tests only. The VAPID pair was generated for this file and protects nothing; the mail provider
        // is a made-up host that the tests intercept, so no key is read from disk and nothing leaves the machine.
        bindings: {
          VAPID_PUBLIC_KEY: 'BKvi_9CXLPzwvJORYASL9Ma6AzgEeBBb5ydVnhpcq99nM_EBjK2ml10egS05bkDcKcNK_W-MvSC_ubFuoxLlZ-0',
          VAPID_PRIVATE_KEY: '-lmEH1xnSBQ7lheI_rL91J5NVbhpIcE7bW3lUmeU_Mw',
          VAPID_SUBJECT: 'mailto:ops@example.test',
          EMAIL_PROVIDER: 'generic',
          EMAIL_API_URL: 'https://mail.example.test/send',
          EMAIL_API_KEY: 'test-key-not-a-real-key',
          EMAIL_FROM: 'SDC <notify@example.test>',
        },
      },
    }),
  ],
  test: { include: ['test/**/*.test.ts'] },
});
