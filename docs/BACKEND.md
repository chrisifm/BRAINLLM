# Resource backend (experimental)

Implemented across 2026-10-29/30. Native resource/auth/media/queue contracts share one project and a fixed loopback
bridge with development and production frontends. `brain run` does not implicitly start or migrate PostgreSQL.
Use the reviewed [PM2/nginx topologies](DEPLOYMENT.md), and migrate existing APIs in behavior-tested slices.

## One contract per edit

```sh
brain add resource posts --fields "title:str! body:str active:bool"
brain add job post_created --fields "resource:str! operation:str! record:obj!"
brain backend schema --json
brain backend migrate --env development
brain backend serve --port 5182 --origin http://127.0.0.1:5182
```

The resource generator writes only implemented files:

```
server/resources/posts/schema.jsbrain  field shapes
server/resources/posts/policy.jsbrain  read/create/update/delete grants
server/resources/posts/hooks.jsbrain   optional durable after-write events
server/jobs/post_created/schema.jsbrain payload shape
server/jobs/post_created/handler.jsbrain server-only async handler
```

Schemas export one literal object (`{ title: 'str!', body: 'str' }`). Types: `str`, `num`, `bool`, `email`, `obj`
and the exact types below; `!` means required. Unknown fields are rejected. Resource and field identifiers are lowercase letters, digits and
underscores, begin with a letter, and are at most 48 bytes; storage-reserved names are rejected. There are 1–64 fields.
Strings are at most 16 KB, emails 254 bytes and objects 32 KB. HTTP JSON bodies are at most 64 KB.

### Exact types, constraints and defaults (2026-10-30)

| Type | Accepted JSON | Stored and returned | Database CHECK |
|---|---|---|---|
| `int` | integer within ±9007199254740991 (JavaScript-safe) | the same number | integral JSON number within the range |
| `decimal(p,s)` | **string** such as `"19.5"`; 1 ≤ p ≤ 38, 0 ≤ s ≤ p | canonical string with exactly `s` decimals (`"19.50"`) | pattern that casts losslessly to `numeric(p,s)` |
| `date` | `YYYY-MM-DD`, years 0001–9999, real calendar day | the same string | pattern |
| `datetime` | RFC 3339 with `Z` or `±HH:MM` offset | UTC `YYYY-MM-DDTHH:MM:SS.sssZ` | pattern |
| `enum(a\|b)` | one listed value (`[A-Za-z0-9_.-]`, 1–64 bytes, ≤64 values) | the same string | `IN (...)` |
| `list<str>` / `list<str,N>` | array of strings, ≤64 items (or N, 1–256), each ≤1024 bytes; required means non-empty | the same array | array of strings and item count |

Decimal transport is a deliberate choice: JSON numbers pass through binary floating point in JavaScript and in
`serde_json`, so money is never accepted as a number. Extra fractional digits are accepted only when they are zeros;
nothing is rounded. `-0` is stored as `0`. Legacy `num` keeps its previous JSON semantics.

A field may use the object form to add a default or single-field uniqueness:
`{ status: { type: 'enum(draft|open)!', default: 'draft' }, slug: { type: 'str!', unique: true } }`.
Defaults are validated against the type at load time and fill a missing value on create and on PUT (PUT replaces the
record). A required field with a default is not required in the OpenAPI request schema. Unknown options are rejected.

An optional `constraints.jsbrain` next to the schema declares composite uniqueness and checks:

```js
<meta server>
export default {
  unique: [['title', 'closes_on']],
  check: [
    { field: 'openings', min: 1, max: 500 },
    { compare: ['salary_min', '<=', 'salary_max'] },
    { together: ['salary_min', 'salary_max', 'currency'] }
  ]
}
```

Composite unique sets have 2–4 fields; single-field uniqueness uses `unique: true`. Ranges apply to `int`, `num`,
`decimal`, `date` and `datetime`; `compare` (`< <= > >= == !=`) needs two ordered fields of the same type; `together`
(2–8 fields) means all present or all absent. At most 16 unique indexes and 16 checks per resource. Each rule is
enforced in the request validator and in Postgres (deterministic `_brain_ck_*` constraints and `_brain_ux_*`
expression indexes). Migration adds missing objects; if existing rows violate one, the migration rolls back with
an actionable message. `verify` reports a missing object as `BDB_SCHEMA_DRIFT`. A duplicate returns HTTP 409
`UNIQUE_CONFLICT` with the field names; a database check violation returns 422 `CONSTRAINT_VIOLATION`.
Legacy contracts serialize and record exactly as before and receive no new database objects. Byte limits of list
items are enforced by the application layer only. `--admin-ui` generates controls only for the legacy types.

Every record read (`GET`, list, search, create, update) now includes `created_at` and `updated_at` as UTC RFC 3339
strings with millisecond precision; they are read-only and documented in OpenAPI.

Policy exports all four operation lists. Default grants are admin-only. Available grants: `admin`, `company`,
`candidate`, `user`, configured role names, `owner` (existing rows only), and `public` (read only). Empty lists deny everyone. There is no implicit
admin bypass. List queries enforce owner filtering in SQL; clients cannot select an owner or supply system columns.
Public reads expose each record's owner UUID; do not grant public read when that identifier is sensitive.

`GET /api/posts?search=words&limit=25&offset=0`, `POST /api/posts`, and `GET|PUT|DELETE /api/posts/<uuid>` use that
contract. PUT replaces the record's data; PATCH changes only the sent fields. List limit: 1–100, offset: 0–10000, search: 256 bytes.
OpenAPI 3.1 JSON is available at `/_brain/api` (admin session required with `--env production`). It documents resource
routes and native account/media/job/link endpoints. CLI/MCP can inspect the same structured source contract;
source saves report conservative contract diffs in `impact` without claiming a running backend was replaced.

Domain writes: [PATCH](backend/patch.md) · [transactions](backend/transactions.md) · [atomic updates](backend/atomic-updates.md) · [batch writes](backend/batch.md) · [state machines](backend/state-machines.md) · [queue policies](backend/queues.md) · [cron](backend/cron.md).

## Opt-in administrative views

```sh
brain add resource posts --fields "title:str! active:bool! score:num meta:obj" --admin-ui
```

Creation emits `/admin/posts`, `/admin/posts/new`, `/admin/posts/:id` and `/admin/posts/:id/edit`.
The private list and detail reads use server loaders; search and pagination use bookmarkable query parameters.
Editors use project DS controls, required-field feedback, busy/error/success states and explicit deletion confirmation.
Boolean and number inputs retain their types; JSON object fields are parsed and validated on the server. Form submissions
support native server-function RPC and the generated no-JavaScript form path. Redirects return to the saved record.

The route guard and generated service require an authoritative administrator session. Native resource policy remains
an additional authority, including denial of an administrator when its grants exclude that role. No private result cache
or client-supplied owner/role is introduced. Configure the fixed backend bridge and migrate before using these routes.

The generator validates the combined project snapshot, acquires bounded file leases and commits backend helpers, routes,
missing DS controls and `client-contract.json` as a recoverable batch with compare-and-swap guards. Individual files
are atomically replaced; a process crash requires journal recovery rather than a filesystem-wide transaction. Only the committed contract reports
`adminUi.status: "generated"`. Existing sources, tokens, fonts and controls are preserved; colliding generated paths fail
before writes. Keep generated views in your application and customize them deliberately. Creating a backend-only resource
remains the default; this flag currently accompanies creation rather than retrofitting an existing contract.

The 30 KiB default route budget is unchanged. The mixed-field acceptance fixture uses a fixed 45-byte application
name and currently reaches that cap exactly: it has no spare budget. This is a bounded reference, not a guarantee for
other names, layouts, fonts or field counts. Build and doctor your application after generation: customized layouts,
controls and field counts can change its route cost. A retained private change journal provides explicit rollback through
the normal changes protocol; generation never automatically installs dependencies, starts Postgres or grants roles.

- Project API keys (scoped, quota-limited, traced): [docs/backend/api-keys.md](backend/api-keys.md)

## Postgres and authentication

Set `DATABASE_URL` in `.brain_env` or the host environment. Values never appear in CLI/MCP reports. There is no
SQLite fallback. `migrate` explicitly creates the system tables and one JSONB table per resource, with owner and GIN
full-text indexes. It serializes migrations with a transaction advisory lock. All request values use bind parameters;
only validated identifiers enter SQL text. A changed field contract is refused until a reviewed data migration
updates its recorded schema. No destructive or implicit schema sync runs during serve.

`brain backend serve` opens two pools: HTTP (`BRAIN_DB_HTTP_POOL`, default 8) and the in-process webhook worker
(`BRAIN_DB_WORKER_POOL`, default 2). At boot it refuses to start (`BDB_POOL_BUDGET`) when
`(http + worker) × BRAIN_BACKEND_PROCESSES` exceeds `max_connections` minus reserved slots and 5 of headroom. Timeouts
are connection startup options, not a session `SET`: `statement_timeout` 5 s, `idle_in_transaction_session_timeout`
10 s, `lock_timeout` 1 s for HTTP and 2 s otherwise, and on Postgres 17+ `transaction_timeout` 9 s for HTTP (workers
and CLI commands are not capped). A transaction-mode pooler must forward the `options` startup parameter; otherwise
set the same values with `ALTER ROLE <app role> SET …`.

- `POST /_brain/auth/signup` accepts `{ email, password }`, creates a candidate, and never accepts a role.
- `POST /_brain/auth/login` accepts the same input and sets an opaque `__Host-brain` cookie: HttpOnly, Secure,
  SameSite=Lax, seven days. The token is hashed in Postgres; roles are loaded from the users table on every request.
- `GET /_brain/auth/me` returns the authenticated user. `POST /_brain/auth/logout` revokes that session.
- Passwords use Argon2id with random salts. Two blocking hash slots prevent unbounded memory use; account lockout
  after ten attempts lasts fifteen minutes. Signup/login/logout events are recorded in Postgres.

Browser writes require an exact configured Origin. Forwarded headers are not trusted. The CLI binds only loopback;
use an HTTPS reverse proxy for a deployed experiment (the session cookie is always Secure). `GET /health` checks
Postgres. Request ids, no-store responses, nosniff, frame denial, CSP and a ten-second request timeout are enabled.
The server uses peer-IP rate limiting (120 burst, one replenished request/second); a proxy currently shares that quota.

Native sessions populate the verified `ctx.user` in Node SSR and the development sidecar through the fixed loopback
bridge; legacy signed sessions never act as a fallback in native mode. JWT uses five-minute access tokens and
rotating persisted refresh families; reuse detection and account mutations revoke active sessions.
Roles and their signup default are declared in `server/auth/roles.jsbrain`; public signup cannot choose a role.
`brain backend bootstrap-admin --user <existing-uuid>` explicitly provisions the first administrator.
Single-use password recovery/email verification and optional Resend delivery are described in [AUTH-FLOWS.md](AUTH-FLOWS.md).
No automatic administrator or production demo login is created. New app display hints are ephemeral, never localStorage authority.

## Transactional hooks and queues

```js
// server/resources/posts/hooks.jsbrain
<meta server>
export default { after_create: 'post_created', after_update: 'post_updated' }
```

Every referenced job must exist. Hook payloads are `{ resource, operation, record }`; a hook job's schema must accept
those fields. The record mutation and queue insertion share a Postgres transaction: enqueue failure rolls back the
mutation. These after-write hooks are asynchronous events. Mandatory `before_create`, `before_update` and
`before_delete` callbacks instead execute before the mutation on every native CRUD entry point; their returned
payload is revalidated. Generate them with `brain add hook` and read [BEFORE-HOOKS.md](BEFORE-HOOKS.md) for the
execution limits, safe rejection contract and same-record CAS conflict protection.

Implement the generated handler before starting a worker; its initial body throws deliberately, so unfinished work
cannot be reported as successful. It receives `{ payload, jobId, attempt, env, backend }`; `ctx.backend` reads and writes resources with a project
API key ([backend/job-backend.md](backend/job-backend.md)). The worker compiles its BrainScript
module graph with the existing compiler and runs it in an owned Node child; it never evaluates job payloads as code.

```sh
brain jobs enqueue post_created --payload '{"resource":"posts","operation":"create","record":{}}' --key example-event
brain jobs work post_created --concurrency 2
brain jobs status <uuid>
brain jobs prune
```

The queue uses `FOR UPDATE SKIP LOCKED`, leases and unique lease tokens, with at-least-once delivery. Make side effects
idempotent by `ctx.jobId`; an expired lease may be delivered again. A stale worker cannot acknowledge a newer lease.
Limits: 10000 active jobs per queue, 1–8 workers per process, no prefetched backlog, 60 seconds per execution, a 256 MB Node old-space heap setting, 90-second
lease, 64 KB payload, default three attempts (library/HTTP allow 1–10), exponential retry delay capped at five minutes.
The final failure or expired final lease becomes `dead`. Errors persisted in job status are generic, never raw handler
exceptions. CLI status and HTTP status omit payloads. Prune explicitly removes at most 1000 finished jobs older than
30 days; idempotency keys expire with those records. Priorities, delays, backoff, longer timeouts, drain and dead-letter
retry are configured per queue (see [queue policies](backend/queues.md)); the values above are the defaults.

Ctrl+C stops claiming work and cancels owned execution. This trusted-project worker is not a sandbox for untrusted
handler code. Node execution costs and delivery semantics must be measured before production adoption.

## Private images

`POST /_brain/uploads` accepts one authenticated multipart image field, checked by content and bounded decode.
PNG/JPEG/WebP originals remain private and unchanged, with separately versioned responsive renditions/crops and
ownership metadata. Image work, input/output bytes, dimensions and decoding memory have explicit budgets.
The media library supports metadata pagination, guarded edits, private downloads and leased deletion cleanup.
See [MEDIA.md](MEDIA.md) for the exact current routes, limits and optional private Google Cloud Storage adapter.
Behavior analytics (collector, privacy, `brain analytics report`) is described in [backend/analytics.md](backend/analytics.md).
General active-content/file formats are not implicitly treated as safe images.
Files of any type (documents, archives, audio, video) use declared buckets: [backend/files.md](backend/files.md).
Running behind a proxy, extra CORS origins and preflight caching: [backend/proxy-cors.md](backend/proxy-cors.md).
Database TLS and boot retry, health probes, backpressure and graceful shutdown: [backend/lifecycle.md](backend/lifecycle.md).
App-declared routes with typed path, query and headers, per-route limits and auth: [backend/routes.md](backend/routes.md).
Signed webhook routes for payment and shipping providers (HMAC, timestamp tolerance, raw body): [backend/webhooks.md](backend/webhooks.md).
Calling external APIs from server logic (timeouts, safe retries, idempotency keys, redacted errors): [backend/outbound.md](backend/outbound.md).

Lists, relations and readers: [relations](backend/relations.md) · [filters](backend/filters.md) · [sorting](backend/sorting.md) · [aggregates](backend/aggregates.md) · [pagination](backend/pagination.md) · [search](backend/search.md) · [ownership](backend/ownership.md) · [visibility](backend/visibility.md) · [slug history](backend/slugs.md).
Server code (`load`, server functions) queries with the same contract through `ctx.backend`: [backend/server-queries.md](backend/server-queries.md).
Keeping numeric ids from another backend as a unique, filterable field: [backend/ids.md](backend/ids.md).

## Remaining acceptance work

Final acceptance uses the combined repository gates and real browser/database evidence, not this feature list.
A source contract change does not automatically migrate deployed data or restart a backend. Registered synchronous
before-write callbacks are distinct from service-side preprocessing: clients allowed by a resource policy can call
its CRUD endpoint directly, so service helpers alone are not an enforced cross-entry-point business hook.
The reference-API throughput/p99 comparison must identify the actual build profile and source. The reproducible
[benchmark harness](../bench/backend/README.md) runs the real reference API with disposable databases; a debug
preflight is not release performance evidence. Authenticated realtime sockets were excluded from the 0.4.0 checkpoint. The development opt-in contract is documented in [Native live resources](NATIVE-LIVE-RESOURCES.md).

## Structured job failure traces

Workers compile private server modules with inline source maps and write a bounded
`_server/trace-index.json` beside the entry module. The index maps generated module
names to application-relative source files; it is not a public build asset.

A failing Node handler reports one `JOB_TRACE` JSON record on its stderr pipe.
It contains the job ID, source locations, generated coordinates, at most 24 frames
across the complete error/cause chain, six causes and explicit truncation flags.
Cause cycles stop traversal. Error messages, names, arbitrary error properties,
variables, payloads, environment values, stdout and raw stack text are excluded.
A formatted or unavailable stack is marked unknown rather than reconstructed from
untrusted text.

The implementation uses [V8 CallSites](https://v8.dev/docs/stack-trace-api) and
[Node source-map lookup](https://nodejs.org/download/release/v22.21.1/docs/api/module.html#sourcemapfindentrylineoffset-columnoffset).
`findEntry` receives zero-based generated coordinates. Only an exact mapped
line whose original source matches the private index supplies an application line.
Original columns remain null with `precision: source-line`; unmapped locations use
`precision: module-only`, a null application line and separate generated
coordinates. Source-map URLs or arbitrary map paths never replace the index path.

The parent worker retains at most 64 KiB of child stderr and continues draining
additional bytes into a fixed-size discard buffer. It accepts only correlated
`JOB_TRACE` records and reconstructs allowlisted paths, coordinates and boolean
flags. A failed attempt emits structured `job-failed` JSON with `jobId`, `queue`,
the sanitized trace, `stderrBytes` and `stderrTruncated`. It never forwards raw
stderr or exception text. Persisted queue errors remain generic.

Waiting and stderr draining run in one joined future. The existing 60-second queue
deadline, kill-on-drop child and owned process-group guard also cover a handler
that floods stderr or leaves a child holding the pipe open. There is no detached
reader task. The index has at most 2048 entries and 256 KiB; the Node trace itself
is limited to 32 KiB. If stderr or tracing is truncated, absence of a location is
not proof that the handler had no relevant frame.
