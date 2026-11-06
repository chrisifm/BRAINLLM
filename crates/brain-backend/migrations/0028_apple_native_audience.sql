-- Native Apple login challenges are isolated from browser login and account notifications.
ALTER TABLE public._brain_auth_challenges ADD COLUMN audience text;
ALTER TABLE public._brain_auth_challenges DROP CONSTRAINT IF EXISTS _brain_auth_challenges_purpose_check;
ALTER TABLE public._brain_auth_challenges ADD CONSTRAINT _brain_auth_challenges_purpose_check
CHECK (purpose IN ('google_state','google_nonce','google_pkce','reset','verify','signup_login','apple_nonce','apple_event','linkedin_state','apple_native_nonce'));
ALTER TABLE public._brain_auth_challenges ADD CONSTRAINT _brain_auth_challenges_audience_check CHECK (
  (purpose = 'apple_native_nonce' AND provider IS NOT DISTINCT FROM 'apple' AND audience IS NOT NULL
    AND length(audience) BETWEEN 3 AND 255
    AND audience ~ '^[A-Za-z0-9-]+([.][A-Za-z0-9-]+)+$')
  OR (purpose <> 'apple_native_nonce' AND audience IS NULL)
);
