-- Aggregate-only document performance. No sample, visitor, session or client timestamp is stored.
CREATE TABLE public._brain_vitals_capacity (
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  rows bigint NOT NULL DEFAULT 0 CHECK (rows BETWEEN 0 AND 250000)
);
INSERT INTO public._brain_vitals_capacity(singleton) VALUES(true);
CREATE TABLE public._brain_vitals_daily (
  day date NOT NULL,
  route_pattern text NOT NULL CHECK (octet_length(route_pattern) BETWEEN 1 AND 160 AND route_pattern ~ '^(/|(/[A-Za-z0-9_-]+|/:[a-z][a-z0-9_]*)+)$'),
  metric text NOT NULL CHECK (metric IN ('LCP','CLS','INP','FCP','TTFB')),
  count bigint NOT NULL CHECK (count BETWEEN 1 AND 1000000000000),
  total double precision NOT NULL CHECK (total>=0 AND total<'Infinity'::float8),
  min double precision NOT NULL CHECK (min>=0 AND min<=3600000),
  max double precision NOT NULL CHECK (max>=min AND max<=3600000 AND (metric<>'CLS' OR max<=1000)),
  good bigint NOT NULL CHECK (good>=0),
  needs_improvement bigint NOT NULL CHECK (needs_improvement>=0),
  poor bigint NOT NULL CHECK (poor>=0),
  PRIMARY KEY(day,route_pattern,metric),
  CHECK (good+needs_improvement+poor=count)
);
