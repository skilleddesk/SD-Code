# cloud/ - the SDC Anywhere relay

A Cloudflare Worker, one Durable Object (the Hub) per computer, and D1. It forwards ciphertext and holds no plaintext.
Nothing here deploys itself: `wrangler deploy` is run by the owner.

```
pnpm --filter @sdc/cloud typecheck
pnpm --filter @sdc/cloud test        # vitest on workerd; the network is replaced by a recorder
```

## Before the first deploy (owner)

1. `wrangler d1 create sdc-anywhere`, put the id in `wrangler.jsonc`, then `wrangler d1 migrations apply sdc-anywhere --remote`.
2. Web Push: `node scripts/vapid.mjs`. Put `VAPID_PUBLIC_KEY` and `VAPID_SUBJECT` in `wrangler.jsonc` `vars`; `wrangler secret put VAPID_PRIVATE_KEY`.
3. Email (OQ-14, which provider owns `H:\emailapi.txt`): set `EMAIL_PROVIDER` (`resend` or `generic`), `EMAIL_FROM` (and `EMAIL_API_URL` for `generic`) as vars, and
   `wrangler secret put EMAIL_API_KEY`. Unset = email off, nothing breaks.
4. In SDC on the computer: Settings -> SDC Anywhere -> the email address.
5. Check once: a sign-in mail to a Gmail address, then "Show original": SPF, DKIM and DMARC all PASS (docs/remote/DNS-RECORDS.md section 6).

Local `wrangler dev` reads `.dev.vars` (gitignored; `.dev.vars.example` lists the names). Secrets are never printed by any script here.

## What is where

| File | Job |
| --- | --- |
| `src/worker.ts` | routes sockets to Hubs; `/api/push/key`, `/api/magic/request`, `/api/magic/redeem`; serves the app with its security headers |
| `src/hub.ts` | the Hub: auth, routing, devices, waiting requests, push -> email escalation, push subscriptions, the pairing offer for a sign-in link |
| `src/webpush.ts` | RFC 8291 encryption, RFC 8292 VAPID, endpoint allowlist |
| `src/email.ts` | provider adapter and the two messages |
| `src/store.ts` | D1: daemons, push subscriptions, email address, magic links, rate limits |
| `migrations/` | D1 schema |
