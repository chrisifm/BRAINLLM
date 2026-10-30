-- Durable payloads are AEAD ciphertext. The shared job contains only this row's opaque reference.
CREATE TABLE public._brain_webhook_events (
    id uuid PRIMARY KEY,
    direction text NOT NULL CHECK(direction IN ('in','out')),
    endpoint text NOT NULL CHECK(endpoint ~ '^[a-z][a-z0-9_]{0,47}$'),
    event_id uuid NOT NULL,
    job_id uuid NOT NULL UNIQUE REFERENCES public._brain_jobs(id) ON DELETE CASCADE,
    queue text NOT NULL CHECK(queue ~ '^[a-z][a-z0-9_]*$' AND length(queue)<=128),
    key_id text NOT NULL CHECK(key_id ~ '^[a-f0-9]{64}$'),
    nonce bytea NOT NULL CHECK(octet_length(nonce)=12),
    ciphertext bytea NOT NULL CHECK(octet_length(ciphertext) BETWEEN 18 AND 16400),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(direction,endpoint,event_id), UNIQUE(key_id,nonce)
);
CREATE INDEX _brain_webhook_retention ON public._brain_webhook_events(created_at,id);
CREATE TABLE public._brain_webhook_attempts (
    event_id uuid NOT NULL REFERENCES public._brain_webhook_events(id) ON DELETE CASCADE,
    attempt integer NOT NULL CHECK(attempt BETWEEN 1 AND 5),
    status integer CHECK(status BETWEEN 100 AND 599),
    code text NOT NULL CHECK(code IN ('accepted','http-rejected','transport-failed','deadline')),
    elapsed_ms integer NOT NULL CHECK(elapsed_ms BETWEEN 0 AND 5000),
    finished_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(event_id,attempt)
);
