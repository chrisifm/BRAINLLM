-- Additive foundation. Existing opaque cookies and 0001/0002 remain unchanged.
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS unaccent WITH SCHEMA public;
ALTER TABLE _brain_users ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE _brain_users ADD COLUMN IF NOT EXISTS disabled_at timestamptz;
ALTER TABLE _brain_users ADD COLUMN IF NOT EXISTS auth_version bigint NOT NULL DEFAULT 0 CHECK (auth_version >= 0);
ALTER TABLE _brain_sessions ADD COLUMN IF NOT EXISTS session_id uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE _brain_sessions ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE _brain_sessions ADD COLUMN IF NOT EXISTS last_used_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE _brain_sessions ADD COLUMN IF NOT EXISTS revoked_at timestamptz;
CREATE UNIQUE INDEX IF NOT EXISTS _brain_sessions_identity ON _brain_sessions(session_id);
CREATE INDEX IF NOT EXISTS _brain_sessions_prune ON _brain_sessions(user_id,expires_at DESC,token_hash);
CREATE INDEX IF NOT EXISTS _brain_sessions_expiry ON _brain_sessions(expires_at,token_hash);
CREATE INDEX IF NOT EXISTS _brain_tokens_expiry ON _brain_auth_tokens(expires_at,token_hash);
CREATE INDEX IF NOT EXISTS _brain_tokens_user_purpose ON _brain_auth_tokens(user_id,purpose);
CREATE INDEX IF NOT EXISTS _brain_limits_expiry ON _brain_login_limits(expires_at,key);
CREATE INDEX IF NOT EXISTS _brain_events_retention ON _brain_auth_events(created_at,id);
CREATE INDEX IF NOT EXISTS _brain_events_user ON _brain_auth_events(user_id,created_at DESC,id);
CREATE INDEX IF NOT EXISTS _brain_uploads_owner_created ON _brain_uploads(user_id,created_at DESC,id);
CREATE INDEX IF NOT EXISTS _brain_jobs_retention ON _brain_jobs(finished_at,id) WHERE state IN ('succeeded','dead');
ALTER TABLE _brain_auth_events ADD COLUMN IF NOT EXISTS outcome text CHECK (outcome IN ('success','denied','failure'));
ALTER TABLE _brain_auth_events ADD COLUMN IF NOT EXISTS request_id uuid;
ALTER TABLE _brain_auth_events ADD COLUMN IF NOT EXISTS metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata)='object' AND octet_length(metadata::text)<=2048);
CREATE TABLE IF NOT EXISTS _brain_identities (
    provider text NOT NULL CHECK (provider IN ('google')), subject text NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    user_id uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE,
    issuer text NOT NULL CHECK (length(issuer)<=255), verified_email text CHECK(length(verified_email)<=254),
    created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(provider,subject)
);
CREATE INDEX IF NOT EXISTS _brain_identities_user ON _brain_identities(user_id);
CREATE TABLE IF NOT EXISTS _brain_refresh_families (
    id uuid PRIMARY KEY, session_id uuid REFERENCES _brain_sessions(session_id) ON DELETE SET NULL,
    user_id uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE, auth_version bigint NOT NULL DEFAULT 0 CHECK(auth_version>=0),
    created_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL, revoked_at timestamptz
);
CREATE INDEX IF NOT EXISTS _brain_refresh_family_expiry ON _brain_refresh_families(expires_at,id);
CREATE INDEX IF NOT EXISTS _brain_refresh_family_session ON _brain_refresh_families(session_id);
CREATE INDEX IF NOT EXISTS _brain_refresh_family_user ON _brain_refresh_families(user_id,expires_at);
CREATE TABLE IF NOT EXISTS _brain_refresh_tokens (
    token_hash text PRIMARY KEY CHECK (length(token_hash)=64), family_id uuid NOT NULL REFERENCES _brain_refresh_families(id) ON DELETE CASCADE,
    parent_hash text UNIQUE REFERENCES _brain_refresh_tokens(token_hash) ON DELETE SET NULL, replacement_hash text REFERENCES _brain_refresh_tokens(token_hash) ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,
    issued_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL, used_at timestamptz
);
CREATE INDEX IF NOT EXISTS _brain_refresh_tokens_family ON _brain_refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS _brain_refresh_tokens_expiry ON _brain_refresh_tokens(expires_at,token_hash);
CREATE TABLE IF NOT EXISTS _brain_auth_challenges (
    token_hash text PRIMARY KEY CHECK(length(token_hash)=64), purpose text NOT NULL CHECK(purpose IN ('google_state','google_nonce','google_pkce','reset','verify')),
    provider text CHECK(provider IN ('google')), user_id uuid REFERENCES _brain_users(id) ON DELETE CASCADE,
    session_id uuid REFERENCES _brain_sessions(session_id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL, consumed_at timestamptz, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS _brain_challenges_expiry ON _brain_auth_challenges(expires_at,token_hash);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_key text;
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_provider text NOT NULL DEFAULT 'filesystem' CHECK(original_provider IN ('filesystem','gcs'));
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_generation text NOT NULL DEFAULT '';
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_bytes bigint CHECK(original_bytes >= 0);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_mime text;
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS original_sha256 text CHECK(original_sha256 IS NULL OR length(original_sha256)=64);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS width integer CHECK(width BETWEEN 1 AND 16384);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS height integer CHECK(height BETWEEN 1 AND 16384);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS alt text CHECK(length(alt)<=1024);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS caption text CHECK(length(caption)<=4096);
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'ready' CHECK(status IN ('staged','ready','deleting','failed'));
ALTER TABLE _brain_uploads ADD COLUMN IF NOT EXISTS version integer NOT NULL DEFAULT 1 CHECK(version>=1);
CREATE TABLE IF NOT EXISTS _brain_media_renditions (
    upload_id uuid NOT NULL REFERENCES _brain_uploads(id) ON DELETE CASCADE,
    version integer NOT NULL CHECK(version>=1), preset text NOT NULL CHECK(length(preset) BETWEEN 1 AND 48),
    provider text NOT NULL DEFAULT 'filesystem' CHECK(provider IN ('filesystem','gcs')), object_key text NOT NULL CHECK(length(object_key) BETWEEN 1 AND 512),
    generation text NOT NULL DEFAULT '', format text NOT NULL CHECK(format IN ('png','jpeg','webp','avif')),
    width integer NOT NULL CHECK(width BETWEEN 1 AND 16384), height integer NOT NULL CHECK(height BETWEEN 1 AND 16384),
    bytes bigint NOT NULL CHECK(bytes>=0), sha256 text NOT NULL CHECK(length(sha256)=64),
    created_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(upload_id,version,preset)
);
-- This outbox survives user/upload cascades. A cascade records work; it cannot delete storage.
CREATE TABLE IF NOT EXISTS _brain_media_deletions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, upload_id uuid NOT NULL,
    provider text NOT NULL CHECK(provider IN ('filesystem','gcs')), object_key text NOT NULL CHECK(length(object_key) BETWEEN 1 AND 512), generation text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(), completed_at timestamptz,
    attempts integer NOT NULL DEFAULT 0 CHECK(attempts>=0), lease_until timestamptz, lease_token uuid, UNIQUE(provider,object_key,generation)
);
CREATE INDEX IF NOT EXISTS _brain_media_deletions_pending ON _brain_media_deletions(created_at,id) WHERE completed_at IS NULL;
CREATE OR REPLACE FUNCTION public._brain_upload_tombstone() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
    INSERT INTO public._brain_media_deletions(upload_id,provider,object_key) VALUES (OLD.id,'filesystem',OLD.id::text||'.png'),(OLD.id,'filesystem',OLD.id::text||'.thumb.png') ON CONFLICT DO NOTHING;
    IF OLD.original_key IS NOT NULL THEN
        INSERT INTO public._brain_media_deletions(upload_id,provider,object_key,generation) VALUES(OLD.id,OLD.original_provider,OLD.original_key,OLD.original_generation) ON CONFLICT DO NOTHING;
    END IF;
    RETURN OLD;
END $$;
CREATE OR REPLACE FUNCTION public._brain_rendition_tombstone() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
    INSERT INTO public._brain_media_deletions(upload_id,provider,object_key,generation) VALUES(OLD.upload_id,OLD.provider,OLD.object_key,OLD.generation) ON CONFLICT DO NOTHING;
    RETURN OLD;
END $$;
CREATE TRIGGER _brain_upload_delete BEFORE DELETE ON _brain_uploads FOR EACH ROW EXECUTE FUNCTION public._brain_upload_tombstone();
CREATE TRIGGER _brain_rendition_delete BEFORE DELETE ON _brain_media_renditions FOR EACH ROW EXECUTE FUNCTION public._brain_rendition_tombstone();
ALTER TABLE _brain_resource_schemas ADD COLUMN IF NOT EXISTS search_fields jsonb NOT NULL DEFAULT '[]'::jsonb;
CREATE OR REPLACE FUNCTION public._brain_search_update() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE field text; textvalue text; weight "char";
BEGIN
    NEW.search_text := ''; NEW.search_vector := ''::tsvector;
    FOREACH field IN ARRAY TG_ARGV LOOP
        IF jsonb_typeof(NEW.data->field)='string' THEN
            textvalue := public.unaccent('public.unaccent'::regdictionary,NEW.data->>field);
            weight := CASE field WHEN 'title' THEN 'A' WHEN 'name' THEN 'A' WHEN 'summary' THEN 'B' ELSE 'D' END;
            NEW.search_text := NEW.search_text || ' ' || lower(textvalue);
            NEW.search_vector := NEW.search_vector || setweight(to_tsvector('simple'::regconfig,textvalue),weight);
        END IF;
    END LOOP;
    NEW.search_text := ltrim(NEW.search_text); RETURN NEW;
END $$;

-- Invalid legacy rows fail validation and roll the entire migration back; never delete them to pass.
ALTER TABLE _brain_jobs ADD CONSTRAINT _brain_jobs_attempts_valid CHECK(attempts>=0 AND attempts<=max_attempts) NOT VALID;
ALTER TABLE _brain_jobs VALIDATE CONSTRAINT _brain_jobs_attempts_valid;
ALTER TABLE _brain_jobs ADD CONSTRAINT _brain_jobs_lease_valid CHECK((state='running' AND lease_until IS NOT NULL AND lease_token IS NOT NULL) OR (state<>'running' AND lease_until IS NULL AND lease_token IS NULL)) NOT VALID;
ALTER TABLE _brain_jobs VALIDATE CONSTRAINT _brain_jobs_lease_valid;
