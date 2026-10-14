-- The owner's second factor for the cloud's own sign-in, and the codes that have been spent (one code works once).
-- A daemon without a secret is what it always was: the sign-in route asks for the address alone (X1).

ALTER TABLE daemons ADD COLUMN totp_secret TEXT;
ALTER TABLE daemons ADD COLUMN mfa_enrolled_at INTEGER;

CREATE TABLE IF NOT EXISTS totp_steps (daemon_id TEXT NOT NULL, step INTEGER NOT NULL, used_at INTEGER NOT NULL, PRIMARY KEY (daemon_id, step));
