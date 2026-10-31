-- Slug history: when a record's slug field changes, the previous slug keeps pointing at the record so a page can
-- answer a permanent redirect to the current slug. TG_ARGV[0] is the slug field. A slug taken again by any record of
-- the same resource stops redirecting (the live slug wins).
CREATE TABLE IF NOT EXISTS public._brain_slug_history(
    resource text NOT NULL,
    slug text NOT NULL,
    record_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(resource, slug)
);
CREATE INDEX IF NOT EXISTS _brain_slug_history_record ON public._brain_slug_history(resource, record_id);
CREATE OR REPLACE FUNCTION public._brain_slug_track() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE v_field text := TG_ARGV[0]; v_resource text := substr(TG_TABLE_NAME, 12); v_previous text; v_current text;
BEGIN
    IF TG_OP = 'UPDATE' THEN v_previous := OLD.data->>v_field; END IF;
    v_current := NEW.data->>v_field;
    IF v_current IS NOT NULL THEN
        DELETE FROM public._brain_slug_history h WHERE h.resource = v_resource AND h.slug = v_current;
    END IF;
    IF v_previous IS NOT NULL AND v_previous IS DISTINCT FROM v_current THEN
        INSERT INTO public._brain_slug_history(resource, slug, record_id) VALUES(v_resource, v_previous, NEW.id)
        ON CONFLICT (resource, slug) DO UPDATE SET record_id = EXCLUDED.record_id, created_at = now();
    END IF;
    RETURN NULL;
END $$;
