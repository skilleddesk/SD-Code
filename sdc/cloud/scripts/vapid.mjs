// Makes a Web Push (VAPID) key pair. Run once:  node scripts/vapid.mjs
//
// The public key goes in wrangler.jsonc as VAPID_PUBLIC_KEY (the page subscribes with it; it is not secret).
// The private key is a Worker secret:  wrangler secret put VAPID_PRIVATE_KEY   (paste the value when asked).
// Nothing is written to disk by this script, so the private half is shown once and is yours to store.

import { generateKeyPairSync } from 'node:crypto';

const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' });
const jwk = privateKey.export({ format: 'jwk' });
const publicKey = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x, 'base64url'), Buffer.from(jwk.y, 'base64url')]).toString('base64url');

console.log(`VAPID_PUBLIC_KEY  (put in wrangler.jsonc vars):\n${publicKey}\n`);
console.log(`VAPID_PRIVATE_KEY (wrangler secret put VAPID_PRIVATE_KEY, then forget it):\n${jwk.d}`);
