# Reading and writing resources from job handlers

A job handler (`server/jobs/<name>/handler.jsbrain`) gets `ctx.backend` with the same calls as server `load()` and
server functions ([server-queries.md](server-queries.md)): `resource(name).list/get/create/update/patch/remove`,
`resource(name).slugRedirect(slug)` and `slugRedirect(name, slug)`.

Jobs have no browser session, so they authenticate with a [project API key](api-keys.md). The key acts as its owner:
its scopes and the owner's role under each resource's policy decide what the job may read or write, before-hooks and
constraints still run, and every call is traced and counted against the key's quotas like any other key request.

## Setup

1. Enable keys (`server/api-keys.jsbrain` `export default { enabled: true }`) and run `brain backend migrate`.
2. Sign in as the account the jobs should act as and create a key with only the scopes they need, for example
   `orders:*` and `products:read`, at `/_brain/keys` (or `POST /_brain/keys`). The secret is shown once.
3. Give the worker both variables, in `.brain_env` or the process environment:

```sh
BRAIN_BACKEND_URL=http://127.0.0.1:5182
BRAIN_JOBS_API_KEY=bk_live_3f9a1c0b7d2e_9c1e…
```

4. Run the backend (`brain backend serve`) and the worker (`brain jobs work order_paid`), or both with `brain backend serve --with-workers`.

`BRAIN_BACKEND_URL` must be a loopback HTTP origin, as for the server runtime. The key is removed from `ctx.env`, so a
handler that logs its environment does not print it. Without both variables, any `ctx.backend.resource(...)` call
throws `Set BRAIN_BACKEND_URL and BRAIN_JOBS_API_KEY ...`, which fails the attempt and retries per the queue policy.

## Stored files: `ctx.backend.files(bucket)`

`files(bucket)` offers `list({limit, offset, record})`, `meta(id)`, `read(id, {maxBytes})` (returns `{bytes, contentType, etag}`, 16 MiB by default),
`upload({filename, contentType, data, record})` and `remove(id)`. The key needs a `files.<bucket>` scope, for example `files.reports:read` or `files.reports:*`. Wildcard
scopes never open a bucket: `*:*` answers `403 API_KEY_SCOPE_DENIED`. The bucket's size, type and quota rules still apply.

## System identity

Some jobs act for nobody: a nightly cleanup, a reconciliation, a job that reads rows of every owner. Give that queue
`runAs: 'system'` in `queue.jsbrain` and a system key created by the operator:

```sh
brain backend keys create --system --user <uuid> --name "Nightly jobs" --scopes orders:*,files.reports:read
```

Set the secret as `BRAIN_JOBS_SYSTEM_KEY` for the worker. A `runAs: 'system'` queue reads only that variable and never
falls back to `BRAIN_JOBS_API_KEY`; other queues never see it. A route handler gets the same choice with
[`runAs`](routes.md#calling-resources-from-a-handler).

| A system key | |
|---|---|
| Bypasses | resource policy and action roles, owner scope, `private` and `readonly` fields, bucket roles |
| Still applies | its scopes (no wildcard resources), `writeonly` fields stay hidden, validation, constraints, state machines, `domain.protect`, before-hooks, quotas, rate limits, revocation, expiry |
| Is traced as | `system: true` on each row of `/_brain/keys/:id/events` and on the request log line |

Only `brain backend keys create --system` can make one: `POST /_brain/keys` has no such field, and a session cannot
rotate a system key (`404`), only revoke it. `brain backend keys rotate` keeps the flag. Its scopes are the whole
boundary, so name the resources and actions the job needs.

## Example: mark an order paid and decrement stock

`server/resources/orders/hooks.jsbrain`:

```js
<meta server>
export default { after_update: 'order_paid' }
```

`server/jobs/order_paid/schema.jsbrain`:

```js
<meta server>
export default { resource: 'str!', operation: 'str!', record: 'obj!' }
```

`server/jobs/order_paid/handler.jsbrain`:

```js
<meta server>
// Delivery is at least once: every write below is safe to repeat for the same jobId.
export default async function handle(ctx) {
  const order = ctx.payload.record
  if (order.data.status !== 'paid' || order.data.fulfilled) return
  const products = ctx.backend.resource('products')
  const page = await products.list({ filter: { sku: { in: order.data.skus } }, limit: 50 })
  for (const product of page.items) {
    await products.patch(product.id, { stock: Math.max(0, product.data.stock - 1) })
  }
  await ctx.backend.resource('orders').patch(order.id, { fulfilled: true })
}
```

The `fulfilled` flag makes a repeated delivery a no-op. For counters that must never double-count, call a declared
[atomic action](atomic-updates.md) instead of read-then-write:
`await ctx.backend.resource('products').action(product.id, 'reserve', { quantity: 1 })`. `.transition(id, name, input)`
does the same for a state transition. The key needs `<resource>:write`, and the action's `roles` still apply to its owner.

## Errors

| Backend answer | What the handler sees |
|---|---|
| `422` field errors | `error.status === 422`, `error.errors` = the field messages |
| `401`/`403`/`404`/`409`/`429` | `error.status` and `error.code` (for example `API_KEY_SCOPE_DENIED`); the message has no response text |
| backend down, timeout (10 s per call), `5xx` | `error.status === 503` |
| `slugRedirect` for an unknown slug | `null` |

An uncaught error fails the attempt; the job retries with the queue's backoff and becomes `dead` after its attempts.
Request bodies are limited to 64 KiB and responses to 2 MiB.

## Tested

`runtime/test/job-backend.test.js` runs the real job runner against a loopback backend (query mapping, the key header,
no cookies or `Origin`, field errors, missing configuration). `crates/brain-backend/tests/api_keys.rs`
(`api_keys_authenticate_scope_quota_trace_rotate_and_revoke`, helper `slug_redirects_and_patch_with_key`) checks a key on PATCH and on slug redirects against Postgres.
