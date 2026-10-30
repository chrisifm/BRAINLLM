CREATE TABLE IF NOT EXISTS _brain_short_links (
    id uuid PRIMARY KEY,
    owner uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE,
    code varchar(80) NOT NULL UNIQUE CHECK (code ~ '^[a-z0-9][a-z0-9-]{0,79}$'),
    target text NOT NULL CHECK (length(target) BETWEEN 1 AND 2048),
    disabled boolean NOT NULL DEFAULT false,
    expires_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS _brain_short_links_owner ON _brain_short_links(owner, created_at DESC);
CREATE INDEX IF NOT EXISTS _brain_short_links_expiry ON _brain_short_links(expires_at) WHERE expires_at IS NOT NULL;
