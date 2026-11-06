# Native document-performance aggregates

Native vitals is an independent opt-in collector for an app's existing native PostgreSQL backend and ordinary
Node/SSR build. It is disabled by default and does not enable behavior analytics, create visitor/session records,
load an external provider, provision infrastructure or upload an operator report.

```json
{
  "vitals": {
    "enabled": true,
    "routes": ["/", "/items/:id"],
    "retentionDays": 30
  }
}
```

Declare only public route patterns. At most 512 distinct patterns are accepted, each at most 160 ASCII characters.
`/` is valid; other patterns use slash-separated literal `[A-Za-z0-9_-]+` segments or named `:[a-z][a-z0-9_]*`
segments. Wildcards, URL/query/fragment values, percent escapes, trailing slashes and dot segments are refused.
Patterns are an operator declaration, not an automatic audit that a route is public. The browser matches its actual
pathname locally, choosing the matching pattern with the most literal segments, then declaration order for ties.
Only that fixed declared pattern is transmitted; the actual pathname, parameters, search and fragment are not.

Configure a fixed loopback `BRAIN_BACKEND_URL` for the selected build environment, migrate the native database and
run the ordinary native backend. Production `brain build` automatically adds the optional module to existing
Node/SSR output. Static output and automatic output with no existing SSR route fail explicitly: enabling this
collector never converts a portable app into a Node app or adds a native server binary. For an externally owned
collector, the existing custom `reportVitals(path)` primitive keeps its existing contract.

The native endpoint is `POST /_brain/vitals`; a frontend base such as `/app/` exposes `/app/_brain/vitals` through
its gateway. Both the development and Node gateways proxy that endpoint without Cookie, Authorization, API key,
referrer, user agent, forwarded IP or client identity headers; Origin and privacy enforcement remain in place.
The development gateway supports the POST transport, but `brain run` does not automatically inject capture in v1.
Automatic capture is provided by production Node/SSR builds.

The v1 collectors support `/` or an app base prefix of at most 256 ASCII bytes using nonempty letters/digits/underscore/hyphen/dot segments, excluding `.` and `..`. Unsupported prefixes fail at build rather than silently disabling browser capture. Existing apps without these opt-ins retain their base behavior.

## Consent and document lifetime

The optional browser module creates performance observers only after `consented('analytics')` is literally true,
with DNT and Global Privacy Control off. It disconnects observers, clears pending values and aborts in-flight fetches
immediately when analytics consent is revoked. Requests omit credentials and referrers, have a three-second abort
budget, use no retry or offline queue and coalesce at most five changed metrics while the document is hidden.
A native 404 disables the active capture lifetime. There is no optional module or bootstrap cost when disabled;
SSR execution never creates capture.

Measurements reuse `vitals(report)` and its supported browser observations. They are initial-document estimates,
not a claim of official metric-library equivalence. Capture stops on BRAINLLM soft navigation or browser history
navigation; it never reassigns the initial document's timings to a later SPA page. Granting consent after the
pathname has changed does not activate that document's capture. A browser may lack a metric entirely.
LCP, INP, FCP and TTFB use milliseconds; CLS is unitless. These observations do not establish an official Core Web
Vitals assessment, exact p75, representative sampling or trusted client telemetry.

## Native boundary and storage

The endpoint accepts at most 4 KiB of JSON:

```json
{"route":"/items/:id","consent":true,"metrics":[{"name":"TTFB","value":42}]}
```

Only `route`, literal boolean `consent:true` and `metrics` are accepted. Metrics contain only `name` and `value`:
one to five unique entries from LCP, CLS, INP, FCP and TTFB, with finite nonnegative values. CLS is capped at 1,000;
other metrics at 3,600,000 ms. Undeclared patterns, duplicate metric names, duplicate body fields, unknown fields,
non-numeric values, queries and the wrong content type are refused. Missing/disabled configuration answers 404.
DNT:1 and Sec-GPC:1 return 204 before any database operation. No session or unrelated API credential is resolved.
Origin protection still applies; a literal consent field is a client declaration, not cryptographic proof of consent.

Migration 0030 creates only daily aggregate rows and one capacity bookkeeping row. The aggregate key is server UTC
day, declared pattern and metric; values are count, sum, minimum, maximum and good/needs-improvement/poor counts.
There is no raw-sample, timestamp-selected-by-client, visitor, session, user, cookie, IP, user-agent, referrer or URL
value column. Anonymous duplicate beacons may count twice because there is no identity-based deduplication.

The shared PostgreSQL rate store allows 600 requests per fixed minute window for one constant anonymous subject.
Only its fixed hash is persisted. The budget is shared across backend processes and can be exhausted by untrusted
clients; it does not imply a visitor count. A quota hit remains consumed when later capacity or transaction work
fails. No batch is retried automatically.

A singleton capacity row is locked in the same transaction as bounded expiration removal, current-key lookup,
capacity reservation and at most five upserts. The global cap is 250,000 aggregate rows, including historical
route declarations. Current declarations changing over time cannot bypass it. Retention is 30 UTC calendar days
by default, configurable from 1 to 90. Physical deletion is lazy and removes at most 5,000 expired keys per accepted
transaction; no exact timed-deletion promise is made. Read-only reports hide expired and no-longer-declared patterns
without running deletion. Database operators writing internal tables directly remain outside this bookkeeping
contract. No backfill, migration-worker or checkpoint state is borrowed.

Ingestion and report database work have a five-second total deadline. Aggregate transactions use a 250 ms lock
budget, a one-second statement budget and a two-second idle-in-transaction budget. Native draining cancels pending
collector requests. Public failures contain fixed codes and fixes without SQL, driver values or body values:
`VITALS_INVALID` (400), `VITALS_TOO_LARGE` (413), `VITALS_RATE_LIMITED` (429), `VITALS_CAPACITY` (503) and
`VITALS_UNAVAILABLE` (503). Existing security/Origin errors retain their own codes.

## Read-only operator report

```sh
brain vitals report --env development --days 7 --json
brain vitals report --days 30 --limit 100 --offset 0 --json
```

The command reads only the selected environment's current database; it does not migrate, change consent, provision
anything or upload results. A production remote database requires verified TLS. It uses a bounded pool connection,
a five-second report budget and bounded cleanup. A SELECT-only database role can run it.

`--days` accepts 1..90 and is clamped to configured retention, `--limit` accepts 1..500 (100 default), and `--offset`
accepts 0..10,000. Groups are ordered by UTC day descending, pattern and metric, with count, mean, minimum, maximum
and category counts/proportions. `nextOffset` and `truncated` describe bounded pagination. `percentileAvailable:false`,
`uniqueVisitorsAvailable:false`, `duplicateBeaconsMayCountTwice:true` and `measurement:"initial-document-estimates"`
make the statistical limits explicit. Category proportions and aggregate means cannot recover exact p75 or unique
visitors. Client estimates can be absent, duplicated or fabricated; the report is not an integrity or compliance claim.
