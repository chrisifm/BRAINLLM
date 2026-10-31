-- Queue liveness: one row per running worker or scheduler process, refreshed while it runs. No job data.
CREATE TABLE IF NOT EXISTS public._brain_workers (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('worker','scheduler')),
    queue text NOT NULL,
    pid integer NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    seen_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS _brain_workers_seen ON public._brain_workers(queue,seen_at DESC);
