-- One record per owner with a role exception. A partial unique index cannot read the owner's role, so the check lives
-- in a trigger, which also covers batch, upsert, import and system-key writes. It reports the same unique violation
-- the indexes do. Arguments: the constraint name, the tenant field or '', then the exempt roles.
CREATE OR REPLACE FUNCTION public._brain_per_owner() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE tenant text := TG_ARGV[1]; taken boolean;
BEGIN
    IF TG_OP='UPDATE' AND NEW.owner=OLD.owner AND (tenant='' OR NEW.data->>tenant IS NOT DISTINCT FROM OLD.data->>tenant) THEN
        RETURN NEW;
    END IF;
    IF EXISTS(SELECT 1 FROM _brain_users u WHERE u.id=NEW.owner AND u.role::text=ANY(TG_ARGV[2:])) THEN
        RETURN NEW;
    END IF;
    -- Two first writes of one owner must not both see an empty table.
    PERFORM pg_advisory_xact_lock(hashtextextended(TG_RELID::text||':'||NEW.owner::text,0));
    EXECUTE format('SELECT EXISTS(SELECT 1 FROM %s WHERE owner=$1 AND id<>$2 AND ($3='''' OR data->>$3 IS NOT DISTINCT FROM $4))', TG_RELID::regclass)
        INTO taken USING NEW.owner, NEW.id, tenant, NEW.data->>tenant;
    IF taken THEN
        RAISE EXCEPTION 'one record per owner' USING ERRCODE='unique_violation', CONSTRAINT=TG_ARGV[0];
    END IF;
    RETURN NEW;
END $$;
