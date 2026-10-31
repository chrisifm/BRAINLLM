-- Queue priorities. Additive and idempotent: existing jobs keep priority 0 and their order.
ALTER TABLE public._brain_jobs ADD COLUMN IF NOT EXISTS priority smallint NOT NULL DEFAULT 0;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid='public._brain_jobs'::regclass AND conname='_brain_jobs_priority') THEN
        ALTER TABLE public._brain_jobs ADD CONSTRAINT _brain_jobs_priority CHECK (priority BETWEEN -100 AND 100);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS _brain_jobs_ready ON public._brain_jobs(queue,priority DESC,available_at,created_at) WHERE state='pending';
CREATE INDEX IF NOT EXISTS _brain_jobs_dead ON public._brain_jobs(queue,finished_at DESC) WHERE state='dead';
