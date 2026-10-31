-- Tombstones: bytes whose row is gone stay recorded until storage confirms they are gone. No existing row changes.
CREATE TABLE IF NOT EXISTS public._brain_file_deletions(
  storage_key text PRIMARY KEY CHECK (length(storage_key) BETWEEN 1 AND 512),
  attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_brain_file_deletions_due ON public._brain_file_deletions(next_attempt_at);
CREATE OR REPLACE FUNCTION public._brain_file_tombstone() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
    INSERT INTO public._brain_file_deletions(storage_key) VALUES (OLD.storage_key) ON CONFLICT DO NOTHING;
    RETURN OLD;
END $$;
DROP TRIGGER IF EXISTS _brain_file_tombstone ON public._brain_files;
CREATE TRIGGER _brain_file_tombstone AFTER DELETE ON public._brain_files FOR EACH ROW EXECUTE FUNCTION public._brain_file_tombstone();
