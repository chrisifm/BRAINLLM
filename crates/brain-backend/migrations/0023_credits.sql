-- Credits: one balance per subject (a user or an organization) and an append-only ledger that explains it.
CREATE TABLE IF NOT EXISTS public._brain_credit_balances(
  subject uuid PRIMARY KEY,
  balance bigint NOT NULL CHECK (balance BETWEEN 0 AND 9007199254740991),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS public._brain_credit_ledger(
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  subject uuid NOT NULL REFERENCES public._brain_credit_balances(subject) ON DELETE RESTRICT,
  delta bigint NOT NULL CHECK (delta<>0),
  balance_after bigint NOT NULL CHECK (balance_after>=0),
  reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 128),
  idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
  actor uuid,
  request text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(subject,idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_brain_credit_ledger_subject ON public._brain_credit_ledger(subject,id DESC);
CREATE OR REPLACE FUNCTION public._brain_credit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'The credit ledger is append-only' USING ERRCODE='55000';
END $$;
DROP TRIGGER IF EXISTS _brain_credit_immutable ON public._brain_credit_ledger;
CREATE TRIGGER _brain_credit_immutable BEFORE UPDATE OR DELETE ON public._brain_credit_ledger FOR EACH ROW EXECUTE FUNCTION public._brain_credit_append_only();
DROP TRIGGER IF EXISTS _brain_credit_no_truncate ON public._brain_credit_ledger;
CREATE TRIGGER _brain_credit_no_truncate BEFORE TRUNCATE ON public._brain_credit_ledger FOR EACH STATEMENT EXECUTE FUNCTION public._brain_credit_append_only();
