-- Preserve explicit opt-out: a trigger with no field arguments must index no values.
CREATE OR REPLACE FUNCTION public._brain_search_update() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE field text; textvalue text; weight "char";
BEGIN
    NEW.search_text := ''; NEW.search_vector := ''::tsvector;
    FOREACH field IN ARRAY coalesce(TG_ARGV,ARRAY[]::text[]) LOOP
        IF jsonb_typeof(NEW.data->field)='string' THEN
            textvalue := public.unaccent('public.unaccent'::regdictionary,NEW.data->>field);
            weight := CASE field WHEN 'title' THEN 'A' WHEN 'name' THEN 'A' WHEN 'summary' THEN 'B' ELSE 'D' END;
            NEW.search_text := NEW.search_text || ' ' || lower(textvalue);
            NEW.search_vector := NEW.search_vector || setweight(to_tsvector('simple'::regconfig,textvalue),weight);
        END IF;
    END LOOP;
    NEW.search_text := ltrim(NEW.search_text); RETURN NEW;
END $$;
