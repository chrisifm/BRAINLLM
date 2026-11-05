-- Display hints are independent of identity, email verification, roles and token claims.
ALTER TABLE public._brain_users ADD COLUMN IF NOT EXISTS display_name text CHECK (length(display_name) <= 200);
ALTER TABLE public._brain_users ADD COLUMN IF NOT EXISTS picture_url text CHECK (octet_length(picture_url) <= 2048);
-- Keep previously shared names, including Apple's one-time authorization name, after upgrading.
UPDATE public._brain_users u SET display_name = (
    SELECT i.display_name FROM public._brain_identities i
    WHERE i.user_id=u.id AND i.display_name IS NOT NULL
    ORDER BY i.provider,i.subject LIMIT 1
) WHERE u.display_name IS NULL AND EXISTS (
    SELECT 1 FROM public._brain_identities i WHERE i.user_id=u.id AND i.display_name IS NOT NULL
);
