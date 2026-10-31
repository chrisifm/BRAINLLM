-- System identity: a key flagged here passes row policy, field access and action roles. Only the operator CLI sets it;
-- every request it makes carries the mark in the trace, so its use can be audited apart from ordinary keys.
ALTER TABLE public._brain_api_keys ADD COLUMN system boolean NOT NULL DEFAULT false;
ALTER TABLE public._brain_api_key_events ADD COLUMN system boolean NOT NULL DEFAULT false;
CREATE INDEX idx__brain_api_key_events_system_at ON public._brain_api_key_events(at) WHERE system;
