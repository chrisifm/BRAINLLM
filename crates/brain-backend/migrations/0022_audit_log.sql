-- Append-only record of mutations. It stores field names, never values, so a private field cannot leak through it.
CREATE TABLE IF NOT EXISTS public._brain_audit_log(
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  at timestamptz NOT NULL DEFAULT now(),
  actor uuid,
  actor_kind text NOT NULL CHECK (actor_kind IN ('user','system_key','server')),
  org uuid,
  resource text NOT NULL CHECK (length(resource) BETWEEN 1 AND 64),
  record uuid,
  operation text NOT NULL CHECK (length(operation) BETWEEN 1 AND 32),
  fields text[] NOT NULL DEFAULT '{}',
  request text CHECK (request IS NULL OR length(request) <= 64)
);
CREATE INDEX IF NOT EXISTS idx_brain_audit_org ON public._brain_audit_log(org,id DESC);
CREATE INDEX IF NOT EXISTS idx_brain_audit_resource ON public._brain_audit_log(resource,record,id DESC);
CREATE OR REPLACE FUNCTION public._brain_audit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'The audit log is append-only' USING ERRCODE='55000';
END $$;
DROP TRIGGER IF EXISTS _brain_audit_immutable ON public._brain_audit_log;
CREATE TRIGGER _brain_audit_immutable BEFORE UPDATE OR DELETE ON public._brain_audit_log FOR EACH ROW EXECUTE FUNCTION public._brain_audit_append_only();
DROP TRIGGER IF EXISTS _brain_audit_no_truncate ON public._brain_audit_log;
CREATE TRIGGER _brain_audit_no_truncate BEFORE TRUNCATE ON public._brain_audit_log FOR EACH STATEMENT EXECUTE FUNCTION public._brain_audit_append_only();
-- Argument: the tenant field, or an empty string. Actor and request come from settings local to the writing transaction.
CREATE OR REPLACE FUNCTION public._brain_audit_row() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  subject jsonb := CASE WHEN TG_OP='DELETE' THEN OLD.data ELSE NEW.data END;
  rec uuid := CASE WHEN TG_OP='DELETE' THEN OLD.id ELSE NEW.id END;
  names text[];
BEGIN
  IF TG_OP='UPDATE' THEN
    SELECT coalesce(array_agg(k ORDER BY k),'{}') INTO names
      FROM jsonb_object_keys(OLD.data||NEW.data) AS k WHERE OLD.data->k IS DISTINCT FROM NEW.data->k;
    -- A rewrite that changes nothing (a search rebuild in a migration) is not a mutation.
    IF cardinality(names)=0 THEN RETURN NULL; END IF;
  ELSE
    SELECT coalesce(array_agg(k ORDER BY k),'{}') INTO names FROM jsonb_object_keys(subject) AS k;
  END IF;
  INSERT INTO public._brain_audit_log(actor,actor_kind,org,resource,record,operation,fields,request) VALUES(
    NULLIF(current_setting('brain.actor',true),'')::uuid,
    coalesce(NULLIF(current_setting('brain.actor_kind',true),''),'server'),
    CASE WHEN TG_ARGV[0]<>'' THEN (subject->>TG_ARGV[0])::uuid END,
    substr(TG_TABLE_NAME,length('_brain_res_')+1),
    rec,
    lower(TG_OP),
    names,
    NULLIF(current_setting('brain.request',true),'')
  );
  RETURN NULL;
END $$;
