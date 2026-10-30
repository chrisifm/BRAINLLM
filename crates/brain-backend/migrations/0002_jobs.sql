CREATE TABLE IF NOT EXISTS _brain_jobs (
    id uuid PRIMARY KEY,
    queue text NOT NULL,
    payload jsonb NOT NULL,
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','running','succeeded','dead')),
    attempts integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL CHECK (max_attempts BETWEEN 1 AND 10),
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_until timestamptz,
    lease_token uuid,
    idempotency_key text,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    UNIQUE(queue,idempotency_key)
);
CREATE INDEX IF NOT EXISTS _brain_jobs_pending ON _brain_jobs(queue,available_at,created_at) WHERE state='pending';
CREATE INDEX IF NOT EXISTS _brain_jobs_leases ON _brain_jobs(queue,lease_until) WHERE state='running';
