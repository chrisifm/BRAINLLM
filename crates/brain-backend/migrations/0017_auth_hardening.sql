-- Replays of a just-rotated refresh token answered inside the grace window; disabled sign-in identities.
ALTER TABLE public._brain_refresh_tokens ADD COLUMN IF NOT EXISTS grace_uses integer NOT NULL DEFAULT 0;
ALTER TABLE public._brain_identities ADD COLUMN IF NOT EXISTS disabled_at timestamptz;
