CREATE TABLE IF NOT EXISTS public._brain_files(
  id uuid PRIMARY KEY,
  bucket text NOT NULL CHECK (bucket ~ '^[a-z][a-z0-9_]{0,47}$'),
  owner uuid NOT NULL REFERENCES public._brain_users(id) ON DELETE RESTRICT,
  resource text CHECK (resource IS NULL OR resource ~ '^[a-z][a-z0-9_]{0,47}$'),
  record uuid,
  field text CHECK (field IS NULL OR field ~ '^[a-z][a-z0-9_]{0,47}$'),
  filename text NOT NULL CHECK (length(filename) BETWEEN 1 AND 255),
  declared_type text NOT NULL CHECK (length(declared_type) BETWEEN 1 AND 255),
  sniffed_type text CHECK (sniffed_type IS NULL OR length(sniffed_type) BETWEEN 1 AND 255),
  content_type text NOT NULL CHECK (length(content_type) BETWEEN 1 AND 255),
  bytes bigint NOT NULL CHECK (bytes >= 0),
  sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  visibility text NOT NULL CHECK (visibility IN ('private','public')),
  storage_key text NOT NULL UNIQUE CHECK (length(storage_key) BETWEEN 1 AND 512),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK ((resource IS NULL) = (record IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_brain_files_bucket_owner ON public._brain_files(bucket,owner,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS idx_brain_files_bucket_created ON public._brain_files(bucket,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS idx_brain_files_record ON public._brain_files(resource,record) WHERE record IS NOT NULL;
