-- Organizations: tenant rows name one organization; a membership gives a user one role in it.
CREATE TABLE IF NOT EXISTS public._brain_orgs(
  id uuid PRIMARY KEY,
  name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  created_by uuid REFERENCES public._brain_users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS public._brain_org_members(
  org_id uuid NOT NULL REFERENCES public._brain_orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES public._brain_users(id) ON DELETE CASCADE,
  role text NOT NULL CHECK (length(role) BETWEEN 1 AND 48),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id,user_id)
);
CREATE INDEX IF NOT EXISTS idx_brain_org_members_user ON public._brain_org_members(user_id,org_id,role);
CREATE TABLE IF NOT EXISTS public._brain_org_invites(
  token_hash text PRIMARY KEY CHECK (token_hash ~ '^[0-9a-f]{64}$'),
  org_id uuid NOT NULL REFERENCES public._brain_orgs(id) ON DELETE CASCADE,
  role text NOT NULL CHECK (length(role) BETWEEN 1 AND 48),
  created_by uuid REFERENCES public._brain_users(id) ON DELETE SET NULL,
  expires_at timestamptz NOT NULL,
  accepted_by uuid REFERENCES public._brain_users(id) ON DELETE SET NULL,
  accepted_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_brain_org_invites_org ON public._brain_org_invites(org_id,expires_at);
-- A tenant row keeps its organization for life and references only records of that same organization.
-- A cross-tenant reference raises the foreign key error of a missing record, so the answer cannot reveal that it exists.
-- Arguments: tenant field, then (reference field, target table, target tenant field, foreign key name) per tenant reference.
CREATE OR REPLACE FUNCTION public._brain_org_row() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  tenant text := TG_ARGV[0];
  org text := NEW.data->>tenant;
  referenced text;
  other text;
  i int := 1;
BEGIN
  IF TG_OP='UPDATE' AND (OLD.data->>tenant) IS DISTINCT FROM org THEN
    RAISE EXCEPTION 'A record cannot move to another organization' USING ERRCODE='23514', CONSTRAINT='_brain_org_immutable';
  END IF;
  WHILE i < TG_NARGS LOOP
    referenced := NEW.data->>TG_ARGV[i];
    IF referenced IS NOT NULL THEN
      EXECUTE format('SELECT data->>%L FROM public.%I WHERE id=$1',TG_ARGV[i+2],TG_ARGV[i+1]) INTO other USING referenced::uuid;
      IF other IS NOT NULL AND other IS DISTINCT FROM org THEN
        RAISE EXCEPTION 'Referenced record not found' USING ERRCODE='23503', CONSTRAINT=TG_ARGV[i+3];
      END IF;
    END IF;
    i := i + 4;
  END LOOP;
  RETURN NEW;
END $$;
