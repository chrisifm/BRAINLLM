-- The typo branch compares every candidate word against `search_text`, so its cost grows with the stored text. Each
-- field now contributes at most its first 256 characters; full-text search keeps indexing the whole value.
CREATE OR REPLACE FUNCTION public._brain_search_update() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE arg text; field text; textvalue text; weight "char"; cfg regconfig := 'simple'::regconfig;
BEGIN
    NEW.search_text := ''; NEW.search_vector := ''::tsvector;
    FOREACH arg IN ARRAY coalesce(TG_ARGV,ARRAY[]::text[]) LOOP
        IF left(arg,1)='=' THEN
            cfg := substr(arg,2)::regconfig;
            CONTINUE;
        END IF;
        field := split_part(arg,':',1);
        IF jsonb_typeof(NEW.data->field)='string' THEN
            textvalue := public.unaccent('public.unaccent'::regdictionary,NEW.data->>field);
            weight := (CASE WHEN position(':' in arg)>0 THEN split_part(arg,':',2)
                ELSE CASE field WHEN 'title' THEN 'A' WHEN 'name' THEN 'A' WHEN 'summary' THEN 'B' ELSE 'D' END END)::"char";
            NEW.search_text := NEW.search_text || ' ' || lower(left(textvalue,256));
            NEW.search_vector := NEW.search_vector || setweight(to_tsvector(cfg,textvalue),weight);
        END IF;
    END LOOP;
    NEW.search_text := ltrim(NEW.search_text); RETURN NEW;
END $$;
