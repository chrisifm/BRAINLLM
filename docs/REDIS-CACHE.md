# Optional shared public SSR cache

The Node production server can share public `<meta cache="N">` page results through standalone Redis.
Opt in with both private runtime environment variables, `REDIS_URL` and `BRAIN_REDIS_NAMESPACE`.
Keep the URL/credentials in the selected private environment profile or host secret store, never in
PUBLIC-prefixed variables, source, logs or build artifacts. A namespace is required: 1..64 ASCII
letters/digits/colon/underscore/hyphen, beginning with a letter or digit. Use a different namespace
for each app. `REDIS_URL` alone leaves the adapter disabled.

The URL accepts `redis` or certificate-validated `rediss`, optional percent-encoded credentials and a
database index 0..1023. Query flags, fragments and certificate-verification bypasses are rejected.
Invalid complete configuration produces a value-free startup warning and falls back to the existing
local cache. Redis Cluster, Sentinel, sessions, jobs, rate limits, socket fan-out, native resource
caching, static export and development SSR are outside this feature.

Only anonymous GET/HEAD requests qualify. Cookie/Authorization headers, including empty ones, Range,
request no-cache/no-store and unknown application/infrastructure headers bypass the cache. A response
with Set-Cookie, any Vary, private/no-store/no-cache, a non-200 status or failed reads is not shared.
Standard browser headers are recognized; if the app varies content by one of them, it must declare
Vary. Do not mark visitor-specific loader results public. Cache keys isolate namespace, build, base,
origin, locale, path and normalized query. They hash those identities rather than logging query values.

Each process admits at most four operations, without a pending queue or retries. DNS/connect/TLS,
authentication and the cache command share a 250 ms deadline. A cold render may await one GET and one
SET, adding at most 500 ms of cache-infrastructure time. Offline servers, authentication/ACL errors,
bad TLS, saturation, malformed data and noeviction/OOM degrade to misses or ignored writes. Closing
or draining the server cancels its owned sockets. Native OS DNS lookups cannot be canceled by destroying
those sockets: an expired lookup retains one of the four admission slots until its callback finishes,
so repeated timeouts cannot queue more resolver work. Late completion cannot open a new connection.
No Redis error body, credential or cached content is logged.

Entries are at most 1 MiB serialized JSON. Freshness is N seconds and stale revalidation/error reuse
is bounded by 2N; shared retention is capped at 24 hours. Larger pages retain local behavior. Stored
timestamps, headers, UTF-8 and TTL are checked before reuse, and each served response gets a fresh
CSP nonce. The existing local LRU and per-process single-flight remain; there is no distributed lock
or promise that several cold processes render only once globally.

The adapter uses only AUTH/SELECT and GET/SET/DEL. It changes no server memory policy, ACL, quota or
unrelated key. Restrict credentials to the app's hashed prefix and required commands. A namespace
prevents accidental collisions; it does not isolate an app from a malicious unrestricted Redis client
or establish a per-namespace memory quota. Redis is trusted infrastructure storing public HTML.

## Measure concurrency before deployment

Keep the supervisor defaults until you measure the actual workload. Compare a fixed dataset/request
mix at 1/2/4 workers, with cold and warm local/Redis caches. Record request p50/p95, failures/misses,
event-loop delay, CPU/RSS and Redis memory/OOM/connection counters. Repeat offline and full-cache
cases. Budget up to four Redis operations per process; excess admission falls through rather than
queueing. Adapter-only throughput on another machine does not establish a safe app worker count.
