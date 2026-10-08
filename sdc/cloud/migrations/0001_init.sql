-- SDC Anywhere relay: account-level data. No content, ever: only what the relay needs to reach a person.
-- Message routing state lives in each Hub (Durable Object), not here.

CREATE TABLE IF NOT EXISTS daemons (
  id TEXT PRIMARY KEY,            -- base64url(first 16 bytes of SHA-256(identity public key))
  pub TEXT NOT NULL,              -- the identity public key, base64url
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS emails (
  daemon_id TEXT PRIMARY KEY REFERENCES daemons(id) ON DELETE CASCADE,
  address TEXT NOT NULL,
  verified_at INTEGER
);

-- Web Push subscriptions, per device. The endpoint is a capability to send that browser a push: treat it as a secret.
CREATE TABLE IF NOT EXISTS push_subscriptions (
  daemon_id TEXT NOT NULL REFERENCES daemons(id) ON DELETE CASCADE,
  device_id TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  p256dh TEXT NOT NULL,
  auth TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (daemon_id, device_id)
);

-- Magic links: the token is stored hashed, valid ten minutes, spent by a button press, never by opening the link.
CREATE TABLE IF NOT EXISTS magic_links (
  token_hash TEXT PRIMARY KEY,
  daemon_id TEXT NOT NULL REFERENCES daemons(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);

CREATE TABLE IF NOT EXISTS rate_limits (
  bucket TEXT PRIMARY KEY,
  window_start INTEGER NOT NULL,
  count INTEGER NOT NULL
);
