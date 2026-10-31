# Running several backend processes

One `brain backend serve` process keeps two kinds of state in its own memory: rate-limit windows and the socket event
fan-out. That is correct for one process. With several processes behind one load balancer (PM2 cluster mode, two
machines, a rolling deploy) each process would count its own window and only its own sockets would hear a write.

Turn on the shared cluster and both move to Postgres:

```js
// server/platform.jsbrain
<meta server>
export default {
  trustProxy: ['127.0.0.1'],
  cluster: { shared: true },
}
```

`BRAIN_BACKEND_PROCESSES` above 1 (the variable the pool budget already uses) turns `shared` on by itself, so a
deployment that declares several processes cannot run with per-process windows by mistake.

`brain backend schema --json` reports the result under `platform.cluster`:

```json
{ "shared": true, "rateStore": "postgres", "socketFanout": "postgres-notify" }
```

## What is shared

| State | One process | `cluster.shared` |
|---|---|---|
| Route `rate` (`'20/min'`, `'5/min per user'`) | in-memory fixed window | one fixed window per route and subject in `_brain_rate_windows`, one atomic upsert per request |
| API-key per-minute limit | in-memory | the same table, scope `api-key` |
| API-key daily/monthly quotas | Postgres | Postgres (unchanged) |
| Sessions, refresh tokens, login lockout | Postgres | Postgres (unchanged) |
| Queue workers | `FOR UPDATE SKIP LOCKED` leases | unchanged: any number of workers |
| Cron | one job per slot through the `cron:<name>:<slot>` idempotency key | unchanged: any number of schedulers |
| Socket invalidation events | local broadcast | local broadcast plus `pg_notify('brain_resource_changes', …)`; every process relays other processes' changes to its sockets |
| Global per-address limiter (120 burst, 1/s) | in-memory | in-memory on purpose: it is a per-process safety net. Put the per-visitor limit at the proxy or use a route `rate` |

The window subject (client address, user id or key id) is stored as a SHA-256 digest, never in clear. Expired windows
are removed every five minutes in batches of 5,000.

## Example: one budget across two processes

```js
// server/routes/quote/route.jsbrain
<meta server>
export default { method: 'GET', path: '/quotes', auth: 'public', rate: '3/min' }
```

With two processes on ports 5182 and 5183 and `cluster: { shared: true }`:

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:5182/quotes   # 200
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:5182/quotes   # 200
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:5183/quotes   # 200
curl -s -i http://127.0.0.1:5183/quotes | head -1                        # HTTP/1.1 429 Too Many Requests
```

```json
{ "error": { "code": "RATE_LIMITED", "fix": "Too many requests to this route; retry after the Retry-After delay" } }
```

Without `shared` the fourth request would succeed: each process would still have its own three requests.

## Socket fan-out

A write committed by process A publishes `{origin, resource, id, owner, operation}` with `pg_notify` after the
transaction commits. Row values never travel. Every other process listens on the channel, ignores its own messages and
delivers the event to its sockets through the same per-socket policy checks as a local write. The listener reconnects
by itself; events sent while it was disconnected are lost, which socket clients already handle: the protocol is an
invalidation hint and a client resynchronizes when told to (`resyncRequired`).

## Cost and limits

- One extra short statement per rate-limited request (route `rate` and API-key requests only) and one `pg_notify` per
  write while sockets are enabled. Size `BRAIN_DB_HTTP_POOL` with that in mind.
- Each process with sockets holds one extra Postgres connection for `LISTEN`.
- A transaction-mode pooler (PgBouncer `pool_mode=transaction`) does not support `LISTEN`; point the backend at
  Postgres directly or at a session-mode pool.
- Windows are fixed, not sliding: a client can send up to twice the budget across a window boundary, as with the
  in-memory limiter.
