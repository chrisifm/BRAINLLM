-- First-party behavior analytics. No IP address, user agent string, form value or free text from the page is stored.
-- Visitors and sessions are random identifiers chosen by the browser; a signed-in request links them to an account.
CREATE TABLE public._brain_analytics_visitors (
    id uuid PRIMARY KEY,
    user_id uuid REFERENCES public._brain_users(id) ON DELETE SET NULL,
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX _brain_analytics_visitors_user ON public._brain_analytics_visitors(user_id) WHERE user_id IS NOT NULL;
CREATE INDEX _brain_analytics_visitors_seen ON public._brain_analytics_visitors(last_seen_at);
CREATE TABLE public._brain_analytics_sessions (
    id uuid PRIMARY KEY,
    visitor uuid NOT NULL REFERENCES public._brain_analytics_visitors(id) ON DELETE CASCADE,
    user_id uuid REFERENCES public._brain_users(id) ON DELETE SET NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    entry_route text CHECK (length(entry_route) <= 256),
    referrer_host text CHECK (length(referrer_host) <= 253),
    utm jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(utm) = 'object' AND length(utm::text) <= 1024),
    country char(2) CHECK (country ~ '^[A-Z]{2}$'),
    device text CHECK (device IN ('mobile','tablet','desktop')),
    language text CHECK (length(language) <= 16)
);
CREATE INDEX _brain_analytics_sessions_visitor ON public._brain_analytics_sessions(visitor, started_at DESC);
CREATE INDEX _brain_analytics_sessions_user ON public._brain_analytics_sessions(user_id, started_at DESC) WHERE user_id IS NOT NULL;
CREATE INDEX _brain_analytics_sessions_started ON public._brain_analytics_sessions(started_at);
CREATE TABLE public._brain_analytics_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    visitor uuid NOT NULL,
    session uuid NOT NULL,
    user_id uuid,
    page integer NOT NULL CHECK (page BETWEEN 0 AND 100000),
    since_ms integer NOT NULL CHECK (since_ms >= 0),
    kind text NOT NULL CHECK (kind ~ '^[a-z][a-z_]{1,15}$'),
    route text NOT NULL CHECK (length(route) BETWEEN 1 AND 256),
    element jsonb CHECK (element IS NULL OR (jsonb_typeof(element) = 'object' AND length(element::text) <= 512)),
    value integer,
    props jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(props) = 'object' AND length(props::text) <= 2048)
);
CREATE INDEX _brain_analytics_events_at ON public._brain_analytics_events(at);
CREATE INDEX _brain_analytics_events_visitor ON public._brain_analytics_events(visitor, at);
CREATE INDEX _brain_analytics_events_session ON public._brain_analytics_events(session, page, since_ms);
CREATE INDEX _brain_analytics_events_user ON public._brain_analytics_events(user_id, at) WHERE user_id IS NOT NULL;
CREATE INDEX _brain_analytics_events_route ON public._brain_analytics_events(route, kind, at);
