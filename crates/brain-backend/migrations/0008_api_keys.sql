-- Project API keys: only a SHA-256 of the high-entropy secret is stored; the plaintext is shown once at creation.
CREATE TABLE public._brain_api_keys(
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES public._brain_users(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    prefix text NOT NULL UNIQUE CHECK (prefix ~ '^[0-9a-f]{12}$'),
    secret_hash text NOT NULL CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    scopes text[] NOT NULL CHECK (cardinality(scopes) BETWEEN 1 AND 32),
    daily_quota bigint CHECK (daily_quota > 0),
    monthly_quota bigint CHECK (monthly_quota > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    expires_at timestamptz,
    revoked_at timestamptz,
    rotated_from uuid REFERENCES public._brain_api_keys(id) ON DELETE SET NULL
);
CREATE INDEX idx__brain_api_keys_user_id ON public._brain_api_keys(user_id);
CREATE INDEX idx__brain_api_keys_rotated_from ON public._brain_api_keys(rotated_from);
-- Atomic per-key counters per UTC day and UTC month.
CREATE TABLE public._brain_api_key_usage(
    key_id uuid NOT NULL REFERENCES public._brain_api_keys(id) ON DELETE CASCADE,
    period text NOT NULL CHECK (period IN ('day','month')),
    starts_on date NOT NULL,
    count bigint NOT NULL DEFAULT 0 CHECK (count >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (key_id, period, starts_on)
);
-- Append-only request trace. Never stores secrets, headers, bodies or query values.
CREATE TABLE public._brain_api_key_events(
    id bigserial PRIMARY KEY,
    key_id uuid NOT NULL REFERENCES public._brain_api_keys(id) ON DELETE CASCADE,
    at timestamptz NOT NULL DEFAULT now(),
    rid text,
    method text NOT NULL,
    route text NOT NULL,
    path text NOT NULL,
    resource text,
    action text,
    status integer NOT NULL,
    latency_ms integer NOT NULL,
    client_ip text,
    user_agent text,
    bytes_out bigint,
    query_keys text[] NOT NULL DEFAULT '{}'
);
CREATE INDEX idx__brain_api_key_events_key_id_id ON public._brain_api_key_events(key_id, id DESC);
CREATE INDEX idx__brain_api_key_events_at ON public._brain_api_key_events(at);
