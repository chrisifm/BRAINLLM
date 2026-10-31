CREATE TABLE IF NOT EXISTS public._brain_rate_windows(
  scope text NOT NULL CHECK (length(scope) BETWEEN 1 AND 128),
  subject bytea NOT NULL CHECK (octet_length(subject)=32),
  window_start timestamptz NOT NULL DEFAULT now(),
  window_ms bigint NOT NULL CHECK (window_ms > 0),
  hits bigint NOT NULL CHECK (hits >= 0),
  PRIMARY KEY (scope,subject)
);
CREATE INDEX IF NOT EXISTS idx_brain_rate_windows_start ON public._brain_rate_windows(window_start);
