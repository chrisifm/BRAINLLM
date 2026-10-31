-- Relations (ref:<resource> fields): keep the real `ref_<field> uuid` foreign-key column in step with the JSONB record.
-- TG_ARGV lists the ref field names. A referential ON DELETE SET NULL arrives as an UPDATE that clears only the column
-- while `data` is unchanged; the field is then removed from `data` so the record and the column never disagree.
CREATE OR REPLACE FUNCTION public._brain_ref_sync() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE field text; patch jsonb := '{}'::jsonb;
BEGIN
    FOREACH field IN ARRAY coalesce(TG_ARGV,ARRAY[]::text[]) LOOP
        IF TG_OP='UPDATE' AND NEW.data IS NOT DISTINCT FROM OLD.data
           AND (to_jsonb(OLD)->>('ref_'||field)) IS NOT NULL AND (to_jsonb(NEW)->>('ref_'||field)) IS NULL THEN
            NEW.data := NEW.data - field;
            NEW.updated_at := now();
        ELSE
            patch := patch || jsonb_build_object('ref_'||field, NEW.data->>field);
        END IF;
    END LOOP;
    IF patch <> '{}'::jsonb THEN
        NEW := jsonb_populate_record(NEW, patch);
    END IF;
    RETURN NEW;
END $$;
