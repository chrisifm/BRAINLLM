# Optional browser error reporting

BRAINLLM can forward consented browser errors to Sentry through its existing Node/SSR server, without a third-party browser SDK. This is an explicit production opt-in:

```json
{ "sentry": { "enabled": true } }
```

Absent, `false`, or `{ "enabled": false }` emits no optional reporter assets and makes no reporting requests. Static export and builds without an existing Node server reject the opt-in. Enabling it never changes a static deployment into a Node deployment. Server exceptions, tracing, profiling, replay, sessions, attachments and breadcrumbs are outside this integration.

The v1 collectors support `/` or an app base prefix of at most 256 ASCII bytes using nonempty letters/digits/underscore/hyphen/dot segments, excluding `.` and `..`. Unsupported prefixes fail at build rather than silently disabling browser capture. Existing apps without these opt-ins retain their base behavior.

## Private server setup

Set `BRAIN_SENTRY_DSN` in the private server environment. Never put a DSN or upload token in `brain.json`, public environment variables or browser code. The server removes `BRAIN_SENTRY_DSN` and `BRAIN_SENTRY_HOST` from the environment exposed to application handlers, including when reporting is off.

The DSN must be canonical HTTPS with a lowercase 32-hex public key and numeric project ID, without a password, query, fragment or encoded path. Hosted ingest targets are supported. A self-hosted target additionally requires its exact approved host and optional port in private `BRAIN_SENTRY_HOST`. Browser input cannot select the upstream destination. Set `PUBLIC_SITE_URL` or the built site origin and preserve the public Host header behind a TLS proxy; forwarded Host/Proto headers do not widen the same-origin boundary.

Missing or invalid configuration prevents listening before any application import or warmup. The startup message contains a fixed error code and correction, never the DSN. The Node tunnel uses verified HTTPS without environment proxies, redirects or retries. Tests use owned local TLS fixtures; they do not establish delivery to a real account.

## Consent and event contents

Capture begins only after literal `consent('analytics', true)`, for a site whose stated analytics purpose includes error reporting. DNT or GPC prevent capture. `consent('analytics', false)` immediately unsubscribes global/framework listeners, aborts pending requests and clears deduplication state. There is no collection before consent, deferred replay or offline queue.

The reporter observes framework-caught errors, window errors and unhandled rejections. A report contains only an allowlisted error class, the fixed value `Application error`, and at most 20 bounded coordinates of owned hashed assets with matching debug IDs. It excludes error messages, raw stacks, arbitrary URLs, queries, referrers, DOM, input, users, sessions, cookies and environment values. Unknown, stale, third-party and core frames are discarded; assets with query strings or fragments are excluded. A report without a mapped owned frame is dropped. The server validates and reconstructs the bounded event instead of forwarding an arbitrary browser envelope.

## Resource limits and delivery

The fixed same-origin endpoint is `POST /_brain/sentry`, under the app base prefix. Limits include a 32 KiB body, four in-flight admissions without a queue, a three-second upstream deadline, per-client and global minute caps, and fixed-memory tracking. These admission and minute limits are per Node process, not a distributed cluster quota. Native DNS work retains its admission slot until the system resolver actually returns, even after the HTTP timeout; a late result cannot open a connection. This bounds resolver concurrency rather than claiming that the platform DNS call can be canceled.

Provider `X-Sentry-Rate-Limits` and 429 cooldowns are honored up to one hour. Failures, overload and cooldown drop events. HTTP 202 acknowledges local processing, not guaranteed provider delivery. Rejected requests use fixed JSON errors; 405 includes `Allow: POST` and overload includes `Retry-After`. No visitor credentials or headers are sent upstream.

## Private maps and deployment

Build artifacts live in `.brain/sentry/<build>/`: matching UUID JavaScript/map pairs, a catalog, and generated browser-JavaScript source artifacts where safe. They never enter `dist/`, static output, precache or archives built from `dist/`. The opt-in build rejects more than 4096 mapped browser modules before startup. Maps contain no `sourcesContent`, original `.brain` text or server functions. Input with inline source-map annotations is not copied as a generated source artifact.

Mapping precision is generated-JavaScript token anchors, not exact BrainScript source lines. Pruned runtime modules map to their final generated JavaScript stage; core and foreign code remain unmapped. Deterministic debug IDs include rewritten generated input and mappings before asset hashing, SRI and compression. Keep each UUID JS/map pair together when using the provider's artifact tooling.

BRAINLLM never uploads artifacts or accepts an upload token. Uploading the matching private artifacts before deployment is a separate explicit operator action. Local mapping and TLS tests do not prove provider ingestion or symbolication.

The shared core observer adds 91 gzip bytes even when reporting is disabled: 3943 to 4034 bytes using the owned minifier and gzip iterations 15, within the 4096-byte core budget. The owned paired fixture measured optional browser JavaScript growth of 4682 minified / 2446 gzip / 2194 Brotli bytes; HTML plus JavaScript grew 5423 raw / 3187 gzip / 2935 Brotli bytes. These are fixture measurements, not a universal per-route cost. Use the generated route ledger for the actual application.
