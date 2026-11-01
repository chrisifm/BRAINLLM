-- When a session last proved its credentials, separate from when it was created; and the one-use grant that lets the login
-- right after a verified signup skip a second captcha.
ALTER TABLE public._brain_sessions ADD COLUMN IF NOT EXISTS auth_time timestamptz;
UPDATE public._brain_sessions SET auth_time=created_at WHERE auth_time IS NULL;
ALTER TABLE public._brain_sessions ALTER COLUMN auth_time SET DEFAULT now();
ALTER TABLE public._brain_sessions ALTER COLUMN auth_time SET NOT NULL;
ALTER TABLE public._brain_auth_challenges DROP CONSTRAINT IF EXISTS _brain_auth_challenges_purpose_check;
ALTER TABLE public._brain_auth_challenges ADD CONSTRAINT _brain_auth_challenges_purpose_check CHECK (purpose IN ('google_state','google_nonce','google_pkce','reset','verify','signup_login'));
