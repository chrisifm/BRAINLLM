-- Operator-owned, bounded data backfills. Identity survives restarts; copied databases may copy it too.
CREATE TABLE public._brain_backfill_identity (
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid()
);
INSERT INTO public._brain_backfill_identity(singleton) VALUES(true);
CREATE TABLE public._brain_data_backfills (
  name text PRIMARY KEY CHECK (name ~ '^[a-z][a-z0-9_]{0,47}$'),
  binding jsonb NOT NULL CHECK (jsonb_typeof(binding)='object'),
  cursor uuid,
  max_id uuid,
  revision bigint NOT NULL DEFAULT 0 CHECK (revision>=0),
  scanned bigint NOT NULL DEFAULT 0 CHECK (scanned>=0),
  written bigint NOT NULL DEFAULT 0 CHECK (written>=0 AND written<=scanned),
  complete boolean NOT NULL DEFAULT false,
  updated_at timestamptz NOT NULL DEFAULT now()
);
