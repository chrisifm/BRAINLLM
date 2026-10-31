# Startup, health, load and shutdown

This page covers how the native backend connects to Postgres, how it reports health, how it behaves under load and
how it stops. The settings live in `server/platform.jsbrain`; every key is optional.

```js
<meta server>
export default {
  limits: {
    // Requests inside the application at once before new ones get 503 OVERLOADED (default 512).
    maxInFlight: 256,
  },
  shutdown: {
    // How long readiness reports DRAINING before the listener stops accepting (default 0, max 60s).
    readyDelay: '5s',
    // Upper bound for open requests to finish after that (default 25s, max 120s).
    drain: '25s',
  },
}
```

Durations accept `500ms`, `5s`, `2m` or a number of milliseconds.

## Connecting to Postgres

**Verified TLS in production.** With `--env production`, a `DATABASE_URL` that points at another machine must use
`sslmode=verify-full`. The Postgres default (`prefer`) encrypts without checking the certificate, so anyone in the
network path can read and change queries. Startup fails with:

```text
BDB_TLS_UNVERIFIED: production DATABASE_URL points at a remote host without sslmode=verify-full; ...
```

Fix it by appending the mode, plus the CA file for a private certificate authority:

```sh
DATABASE_URL='postgres://shop@db.shop.example:5432/shop?sslmode=verify-full&sslrootcert=/etc/shop/db-ca.pem'
```

Loopback addresses (`127.0.0.0/8`, `::1`, `localhost`) and unix sockets (`?host=/var/run/postgresql`) are allowed
without TLS. Development keeps accepting any mode. The check applies to every CLI command that opens the database.

**Boot retry.** `brain backend serve` and `brain jobs work` start even if Postgres is still coming up. A refused
connection, a server that is starting (`57P03`) or full connection slots (`53300`) are retried with full-jitter
backoff: a random delay up to 250 ms, doubling per attempt up to 5 s, for at most 60 s. Each retry logs
`{"event":"database-connect-retry","attempt":n,"delayMs":ms}` without the URL. Wrong credentials, a missing database
or a TLS failure stop immediately. After 60 s the command fails with `BDB_CONNECT_TIMEOUT`. One-shot commands
(`migrate`, `bootstrap-admin`, media) do not retry.

## Health probes

| Route | Use it for | `200` | `503` |
|---|---|---|---|
| `GET /health/live` | process supervisor, container liveness | the process answers | never |
| `GET /health/ready` | load balancer, container readiness | Postgres answered within 1 s | `DRAINING`, `OVERLOADED`, `DATABASE_UNAVAILABLE` |
| `GET /health` | existing setups | Postgres answered | `DATABASE_UNAVAILABLE` |

```sh
curl -s http://127.0.0.1:5182/health/ready -H 'Accept: application/json'
# {"ok":true,"schemaVersion":1,"status":"ready"}
```

Liveness never touches the database: restarting a process because Postgres is slow only makes an outage longer.
Probes are not counted against `maxInFlight`, so they keep answering under load.

## Backpressure

Two signals turn overload into a fast, retryable answer instead of a slow timeout:

- **In-flight cap.** When `maxInFlight` requests are already inside the application, a new one gets `503` with
  `Retry-After: 1` and code `OVERLOADED`. Readiness reports `OVERLOADED` at the same time.
- **Pool wait.** A request waits at most 2 s for a database connection. If it gives up, the response is `503` with
  `Retry-After: 2` and code `OVERLOADED`, not `500`.

```json
{"schemaVersion":1,"rid":"…","error":{"code":"OVERLOADED","fix":"The server is at capacity; retry after the Retry-After delay","fields":null}}
```

Clients should retry idempotent requests after the `Retry-After` delay. A shop front end, for example, can retry a
`GET /api/products` once and show a "busy, try again" message for a failed checkout instead of an error page.

## Shutdown

On `SIGTERM` or `Ctrl-C` the backend:

1. marks itself as draining: `/health/ready` answers `503 DRAINING` with `Retry-After: 5`, liveness stays `200`;
2. keeps serving for `readyDelay`, so the load balancer sees the failed readiness and stops sending traffic;
3. closes the listener and native sockets, then waits up to `drain` for open requests;
4. logs `{"event":"drain-timeout","inFlight":n}` if requests are still open after `drain`, and stops anyway;
5. stops the webhook worker and closes the Postgres pool.

Set `readyDelay` to at least your load balancer's readiness interval times its failure threshold (for example 2 s × 2
→ `5s`). Without a load balancer leave it at `0`.

## Tested example

`crates/brain-backend/tests/edge.rs` holds the only in-flight slot with a request whose body never arrives, checks
that the next request gets `503 OVERLOADED` with `Retry-After: 1`, that readiness reports overload while liveness
stays up, that the slot is released when the request is cancelled, and that draining fails readiness. A second test
exhausts a small pool and checks that a list request answers `503 OVERLOADED` instead of `500`.
`crates/brain-backend/src/boot.rs` tests the TLS rule for remote, loopback and unix-socket URLs and the retry budget.
