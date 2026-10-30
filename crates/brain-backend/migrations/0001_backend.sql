CREATE TABLE IF NOT EXISTS _brain_users (
    id uuid PRIMARY KEY,
    email text NOT NULL UNIQUE,
    password_hash text NOT NULL,
    role text NOT NULL CHECK (role IN ('admin', 'company', 'candidate')),
    verified boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS _brain_sessions (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS _brain_sessions_user ON _brain_sessions(user_id);
CREATE TABLE IF NOT EXISTS _brain_auth_tokens (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE,
    purpose text NOT NULL CHECK (purpose IN ('reset', 'verify')),
    expires_at timestamptz NOT NULL
);
CREATE TABLE IF NOT EXISTS _brain_auth_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id uuid REFERENCES _brain_users(id) ON DELETE SET NULL,
    event text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS _brain_uploads (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES _brain_users(id) ON DELETE CASCADE,
    media_type text NOT NULL,
    bytes bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS _brain_resource_schemas (
    name text PRIMARY KEY,
    fields jsonb NOT NULL
);
CREATE TABLE IF NOT EXISTS _brain_login_limits (
    key text PRIMARY KEY,
    attempts integer NOT NULL,
    expires_at timestamptz NOT NULL
);
