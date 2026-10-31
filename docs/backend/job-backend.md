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

4. Run the backend (`brain backend serve`) and the worker (`brain jobs work order_paid`).

`BRAIN_BACKEND_URL` must be a loopback HTTP origin, as for the server runtime. The key is removed from `ctx.env`, so a
handler that logs its environment does not print it. Without both variables, any `ctx.backend.resource(...)` call
throws `Set BRAIN_BACKEND_URL and BRAIN_JOBS_API_KEY ...`, which fails the attempt and retries per the queue policy.

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

The `fulfilled` flag makes a repeated delivery a no-op. For counters that must never double-count, prefer an
[atomic update](atomic-updates.md) route over read-then-write.

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
