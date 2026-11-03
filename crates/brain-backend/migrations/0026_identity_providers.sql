-- Sign in with Apple and Sign In with LinkedIn join Google as identity providers: their linked accounts, their one-use
-- browser nonces and redirect states, the de-duplication of Apple's signed account notifications (`apple_event`, keyed
-- by the notification id), and the display name a provider shares (Apple only on the first authorization).
ALTER TABLE public._brain_identities DROP CONSTRAINT IF EXISTS _brain_identities_provider_check;
ALTER TABLE public._brain_identities ADD CONSTRAINT _brain_identities_provider_check CHECK (provider IN ('google','apple','linkedin'));
ALTER TABLE public._brain_identities ADD COLUMN IF NOT EXISTS display_name text CHECK (length(display_name) <= 200);
ALTER TABLE public._brain_auth_challenges DROP CONSTRAINT IF EXISTS _brain_auth_challenges_provider_check;
ALTER TABLE public._brain_auth_challenges ADD CONSTRAINT _brain_auth_challenges_provider_check CHECK (provider IN ('google','apple','linkedin'));
ALTER TABLE public._brain_auth_challenges DROP CONSTRAINT IF EXISTS _brain_auth_challenges_purpose_check;
ALTER TABLE public._brain_auth_challenges ADD CONSTRAINT _brain_auth_challenges_purpose_check CHECK (purpose IN ('google_state','google_nonce','google_pkce','reset','verify','signup_login','apple_nonce','apple_event','linkedin_state'));
