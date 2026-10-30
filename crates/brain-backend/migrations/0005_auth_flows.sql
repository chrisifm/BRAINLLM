-- Token plaintext is never persisted. Legacy unbound tokens remain unusable until expiry/cleanup.
ALTER TABLE public._brain_auth_tokens ADD COLUMN email_bound text;
ALTER TABLE public._brain_auth_tokens ADD COLUMN auth_version bigint CHECK(auth_version>=0);
ALTER TABLE public._brain_auth_tokens ADD COLUMN consumed_at timestamptz;
-- Sending is confirmed before activation. Unconfirmed/timeout deliveries cannot authorize a mutation.
ALTER TABLE public._brain_auth_tokens ADD COLUMN activated_at timestamptz;
ALTER TABLE public._brain_auth_tokens ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX _brain_auth_tokens_live ON public._brain_auth_tokens(user_id,purpose,created_at DESC) WHERE consumed_at IS NULL;
CREATE TABLE public._brain_auth_roles (
    role text PRIMARY KEY CHECK(role ~ '^[a-z][a-z0-9_]{0,47}$' AND role NOT IN ('public','owner'))
);
INSERT INTO public._brain_auth_roles(role) VALUES('admin'),('company'),('candidate'),('user');
-- Baseline adoption already proves this exact legacy allowlist. Catalog names are quoted with %I.
DO $$ DECLARE legacy_name text;
BEGIN
    FOR legacy_name IN SELECT conname FROM pg_constraint WHERE conrelid='public._brain_users'::regclass
        AND contype='c' AND convalidated AND replace(pg_get_constraintdef(oid),' ','')=
        'CHECK((role=ANY(ARRAY[''admin''::text,''company''::text,''candidate''::text])))'
    LOOP EXECUTE format('ALTER TABLE public._brain_users DROP CONSTRAINT %I',legacy_name); END LOOP;
END $$;
ALTER TABLE public._brain_users ADD CONSTRAINT _brain_users_role_registered
    FOREIGN KEY(role) REFERENCES public._brain_auth_roles(role) ON DELETE RESTRICT;
