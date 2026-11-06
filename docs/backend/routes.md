# Application routes

Resources cover CRUD. When an app needs its own endpoint (a quote calculator, a checkout step, a
provider callback), declare it as a route folder:

```
server/routes/<name>/route.jsbrain     contract: method, path, auth, input schema, limits
server/routes/<name>/handler.jsbrain   export default async function handle(ctx) { ... }
```

`<name>` is a lowercase identifier. `brain backend serve` reads every contract, compiles every handler
and probes its default export before HTTP starts; a contract error or a missing handler stops the boot
with the file path in the message.

## Same origin

`brain build` lists every declared route in `_server/manifest.json`, and the gateways (`brain start`, `brain run`,
the generated `server.mjs`) proxy exactly those method and path pairs to the backend. The raw body crosses byte for
byte up to the route's `bodyLimit`, and the response keeps its status, its `x-*` headers and its `cache-control`. Any
other path stays on the frontend, so a gateway never becomes an open proxy to the backend. `brain check` warns
B0750 when a route would hide a view, a literal `public/` file or a framework prefix, or starts with a parameter. The
gateway does not proxy that route until it is renamed, so the page or file keeps answering.

## Contract

```js
// server/routes/order_quote/route.jsbrain
<meta server>
export default {
  method: 'POST',                         // GET, POST, PUT, PATCH, DELETE (default GET)
  path: '/orders/{id}/quote',             // {name} segments are path parameters
  auth: 'user',                           // 'public', 'user' (default), 'internal' or ['admin', 'support']
  params: { id: 'uuid' },                 // str (default), int, bool, uuid; a bad value answers 404
  query: { currency: 'str!', rush: 'bool' }, // '!' = required; unknown keys answer 422
  headers: ['x-shop-channel'],            // forwarded to the handler; everything else is dropped
  body: 'json',                           // 'json' (default) or 'raw'
  bodyLimit: '16kb',                      // default 64 KiB, at most 1 MiB
  timeout: '5s',                          // default 10s, at most 30s
  rate: '20/min per user',                // per IP by default; ' per user' needs auth
}
```

Paths must not use the reserved prefixes `/_brain`, `/api`, `/health` and `/s`. At most 128 routes,
32 query parameters and 16 forwarded headers per route. `cookie` and `authorization` are never
forwarded: the backend authenticates the request itself.

`brain check` warns B0816 when a handler reads a literal header absent from its sibling contract,
including proven aliases such as `const h = ctx.headers`. Use the lowercase header name and explicitly
add a safe header to `headers` when needed; authentication uses `ctx.user`. The checker never forwards
headers automatically and skips evidence obscured by unknown keys, mutation or escaping references.

## Provider-specific signature verification

For a provider whose signature protocol is not HMAC, declare `body: 'raw', auth: 'public',
verify: { scheme: 'custom' }` and add a private sibling `verify.jsbrain`:

```js
<meta server>
import verifyProvider from '../../lib/verify-provider.jsbrain'
export default async function verify(ctx) {
  return await verifyProvider(ctx.rawBody, ctx.rawBodyEncoding, ctx.headers, ctx.env)
}
```

The default export must resolve to the boolean `true` before the handler can run. It receives only
`rawBody`, `rawBodyEncoding`, explicitly declared headers and the selected environment. It has no
visitor identity, backend client or system-key authority. Route backend credentials are removed from
the verifier's environment. B0816 checks literal header reads in both siblings. Every declared header
must occur once on the incoming request; duplicate values are rejected before verification on every gateway.

The application verifier implements the provider's actual protocol: bind the signature to these exact
body bytes, validate its timestamp and certificate/key chain, pin permitted key endpoints, bound key
fetches and persist replay protection when required. A caller-supplied certificate URL is not a trusted
key source. Returning a truthy object, decoding a token without verifying it, or merely fetching a
certificate never authenticates an event. There is no generic provider verification shortcut.

Custom verification is available only for raw public POST, PUT, PATCH and DELETE routes. It replaces
the browser Origin requirement with successful cryptographic verification; it does not weaken Origin
checks on ordinary public writes. False or non-boolean results answer `401 SIGNATURE_INVALID`;
verifier failures answer `503 VERIFIER_UNAVAILABLE`. Verification and the handler share the route's
single absolute timeout, including queue/executor admission; expiry answers `504 ROUTE_TIMEOUT` and
terminates the owned sidecar invocation. The backend refuses startup if the verifier is missing or is
not callable. Keep verification inexpensive and set bounded provider timeouts inside that total budget.

## Handler

```js
// server/routes/order_quote/handler.jsbrain
<meta server>
export default async function handle(ctx) {
  if (ctx.body.items < 1) throw { errors: { items: 'add at least one item' } }
  const total = ctx.body.items * 5
  return { status: 201, body: { order: ctx.params.id, total, currency: ctx.query.currency },
           headers: { location: `/orders/${ctx.params.id}/quote` } }
}
```

`ctx` carries `rid`, `route`, `method`, `path`, `params`, `query`, `headers`, `user` (`{id, role, verified}` or
`null`), `client` (the resolved client IP), `env` (the resolved environment), `body`, and for raw or signed routes `rawBody` with
`rawBodyEncoding` (`utf8` or `base64`) plus `verified` (see [webhooks.md](webhooks.md)).

Return either a plain value (answered as `200` JSON) or `{status, body, headers}`. A returned `status` from 200 to 599
is answered as written, 5xx included. A number outside that range is a handler bug: it answers
`500 ROUTE_INVALID_STATUS` and logs a `route-invalid-status` event, never a silent `200`. An object whose only key is a
non-numeric `status` is plain data. Response headers are limited to `location`, `cache-control`, `etag`, `retry-after`, `link`, `content-language` and `x-*`.

To answer a file built by the handler, return `{ file: { base64, name, type } }` (optionally with `status` and `headers`):
`return { file: { base64: pdf.toString('base64'), name: 'report.pdf', type: 'application/pdf' } }`. It is always an
attachment with `nosniff` and a `default-src 'none'` policy, as a stored file is; `Content-Disposition` cannot be set
any other way. Invalid base64, a missing `name` or a non-2xx status answers `500 ROUTE_INVALID_STATUS`.

## Calling resources from a handler

Declare `runAs` in the contract to give the handler `ctx.backend`, with `resource(name)`, `files(bucket)` and
`slugRedirect(name, slug)` as in [job handlers](job-backend.md):

| `runAs` | Key read from the backend's environment |
|---|---|
| absent | none: `ctx.backend` is not present |
| `'key'` | `BRAIN_ROUTES_API_KEY` |
| `'system'` | `BRAIN_ROUTES_SYSTEM_KEY` ([system identity](job-backend.md#system-identity); no fallback) |

The call acts as the key, not as `ctx.user`: check the caller yourself before reading on their behalf, above all with
`runAs: 'system'`. Neither variable appears in `ctx.env`.

## Source rules

Handlers follow the same source rules as the rest of `server/`: one default export; no `class`, `while`, `do` or
classic `for`. `for (const x of list)` is allowed. `brain check`, `brain build` and `brain doctor` report them, and
`brain backend serve` and `brain jobs work` refuse to start on them. A handler with no default export (B0772) or a
secret-shaped literal in its source (B0773) is refused the same way; read keys from `ctx.env`. `brain check` warns B0812 when a handler reads `process.env`, and when it reads a
variable that `.brain_env` declares only under `[production]`, which the route reads as unset in development.

Throwing `{errors: {field: 'message'}}` answers `422 INVALID_INPUT` with those fields. Throwing an error
with a 4xx `status` answers that status with a generic `ROUTE_REJECTED` code, and `throw notFound()` or `throw gone()` answers `404` or `410` the same way.
`redirect(url, status)`, returned or thrown, answers that redirect with `Location`. Any other throw answers
`500 ROUTE_FAILED`; its message is never sent to the client. The server log line `route-failed` names the error class
(`error`), the app file and line that threw (`location`), and, when the backend client raised it, the backend's
`status`, `code` and field names (`backend`); never the message or a value. `scripts/gates/http-truth-table.mjs` pins every shape on
the backend and on each gateway.

## Checks the backend runs before the handler

In order: IP rate limit → access (`401`/`403`) → user rate limit (`429` with `Retry-After`) → typed
path parameters (`404`) → strict query (`422` with `fields.query`) → body limit (`413 BODY_TOO_LARGE`)
→ signature (`401`) → content type (`415 JSON_REQUIRED` unless `body: 'raw'`) → JSON parsing (`400
INVALID_JSON`; keys `__proto__`, `constructor` and `prototype` are refused at any depth).

Mutations on unsigned routes keep the browser `Origin` check of the rest of the backend. A handler that
outlives `timeout` answers `504 ROUTE_TIMEOUT`; when every executor is busy the route answers `503
OVERLOADED` at once (requests are not queued), and the `backend-request-failed` line carries `"code":"OVERLOADED"`.

Handlers run in private Node sidecars (four per server) with the selected environment's variables, so
they can use `fetch` and any server module of the app.

## OpenAPI

`GET /_brain/api` (admin-only with `--env production`; any other process warns B0771 at start) includes every route as an
operation `route_<name>` with its path, query and header parameters, request body, error responses and an
`x-brain-route` extension (limits, rate, signature header). File buckets appear as
`files_<bucket>_upload`, `_download`, `_delete` and `_meta` with an `x-brain-bucket` extension. Signing
secrets never appear in the document.

## Internal routes

For machines and operators, not browsers:

```js
export default {
  method: 'POST',
  path: '/ops/reindex',
  auth: 'internal',
  internal: { secret: ['INTERNAL_SECRET', 'INTERNAL_SECRET_OLD'], header: 'x-internal-secret', loopback: true },
}
```

`secret` names one environment variable, or up to 4 for rotation, each at least 32 bytes: a short value stops startup, a missing one
disables the route (503, no handler runs), and `brain check` warns about both first (B0790). `header` defaults to `x-internal-secret` and cannot be `cookie` or `authorization`. The comparison is HMAC
verification under a per-process random key, so its time does not depend on where the bytes differ. A wrong secret
answers 403, and a session is never accepted in its place. No browser Origin is required, and `rate ... per user` is
rejected because there is no user.

`loopback: true` also requires the connection itself and the resolved client to be local, with no `X-Forwarded-For` or
`Forwarded` header, so a proxy on the same machine cannot speak for a remote caller. The site server does not forward the
secret header, so these routes are reached directly on the backend port, not through the public site.
