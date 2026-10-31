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

## Contract

```js
// server/routes/order_quote/route.jsbrain
<meta server>
export default {
  method: 'POST',                         // GET, POST, PUT, PATCH, DELETE (default GET)
  path: '/orders/{id}/quote',             // {name} segments are path parameters
  auth: 'user',                           // 'public', 'user' (default) or ['admin', 'support']
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

`ctx` carries `rid`, `route`, `method`, `path`, `params`, `query`, `headers`, `user` (`{id, roles}` or
`null`), `client` (the resolved client IP), `body`, and for raw or signed routes `rawBody` with
`rawBodyEncoding` (`utf8` or `base64`) plus `verified` (see [webhooks.md](webhooks.md)).

Return either a plain value (answered as `200` JSON) or `{status, body, headers}`. Response headers are
limited to `location`, `cache-control`, `etag`, `retry-after`, `link`, `content-language` and `x-*`.

Throwing `{errors: {field: 'message'}}` answers `422 INVALID_INPUT` with those fields. Throwing an error
with a 4xx `status` answers that status with a generic `ROUTE_REJECTED` code. Any other throw answers
`500 ROUTE_FAILED`; its message is never sent to the client.

## Checks the backend runs before the handler

In order: IP rate limit → access (`401`/`403`) → user rate limit (`429` with `Retry-After`) → typed
path parameters (`404`) → strict query (`422` with `fields.query`) → body limit (`413 BODY_TOO_LARGE`)
→ signature (`401`) → content type (`415 JSON_REQUIRED` unless `body: 'raw'`) → JSON parsing (`400
INVALID_JSON`; keys `__proto__`, `constructor` and `prototype` are refused at any depth).

Mutations on unsigned routes keep the browser `Origin` check of the rest of the backend. A handler that
outlives `timeout` answers `504 ROUTE_TIMEOUT`; when every executor is busy the route answers `503
OVERLOADED`.

Handlers run in private Node sidecars (four per server) with the selected environment's variables, so
they can use `fetch` and any server module of the app.
