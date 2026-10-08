-- Sign-in by email looks a daemon up by its address; expiry cleanup looks links up by time.
-- Addresses are stored lower-cased by the Worker, so the comparison is a plain equality.

CREATE INDEX IF NOT EXISTS emails_by_address ON emails(address);
CREATE INDEX IF NOT EXISTS magic_links_by_expiry ON magic_links(expires_at);
