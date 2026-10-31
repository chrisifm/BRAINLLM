# Outbound calls from server logic

Route handlers, jobs and server modules call external APIs (payment providers, shipping rates, mail
services) with `httpCall`. It is auto-imported like the other native helpers; no package is installed.

```js
// server/routes/checkout/handler.jsbrain
<meta server>
export default async function handle(ctx) {
  const charge = await httpCall('https://payments.example.test/v1/charges', {
    method: 'POST',
    headers: { authorization: `Bearer ${process.env.PAYMENTS_API_KEY}` },
    json: { amount: ctx.body.total, currency: 'usd', order: ctx.body.order },
    idempotencyKey: `order-${ctx.body.order}`, // same key on every retry of this charge
    timeout: 8000,
  })
  if (!charge.ok) throw { errors: { payment: 'the card was declined' } }
  return { status: 201, body: { charge: charge.json.id } }
}
```

## Options

| Option | Default | Meaning |
|---|---|---|
| `method` | `GET` | HTTP method |
| `headers` | `{}` | Request headers |
| `json` | — | Serialized as the body with `content-type` and `accept` set to JSON |
| `body` | — | Raw body (string, `Uint8Array`, `URLSearchParams`) when `json` is not used |
| `timeout` | `10000` | Milliseconds per attempt, 1..60000 |
| `retries` | `2` | Extra attempts, 0..5 |
| `idempotencyKey` | — | `true` generates a UUID; a string is sent as is. Sent as `Idempotency-Key` on every attempt |
| `maxBytes` | `1048576` | Response size cap, at most 16 MiB |
| `backoff` | `250` | Base delay in ms for full-jitter exponential backoff (capped at 5 s) |
| `allowPrivate` | `false` | Allow loopback, private and link-local targets (internal services) |
| `signal` | — | `AbortSignal` to cancel the call |

## Retries

Only replayable requests retry: `GET`, `HEAD`, `OPTIONS`, `PUT`, `DELETE`, or any method that carries an
idempotency key. A `POST` without a key is sent once, so a charge is never duplicated by a retry.
Retries happen on network errors, timeouts and statuses 408, 425, 429, 500, 502, 503 and 504. A
`Retry-After` header (seconds or date, capped at 30 s) replaces the backoff delay.

## Result and errors

`httpCall` resolves to `{status, ok, headers, text, json, attempts}` for every answer that is not
retryable, including 4xx, so the handler decides what a 404 or 409 means. `json` is `null` unless the
response is JSON. Redirects are not followed.

It throws `HttpCallError` with `code` (`TIMEOUT`, `NETWORK`, `HTTP_STATUS`, `TOO_LARGE`, `UNSAFE_URL`,
`ABORTED`), `status`, `attempts` and `url`. Messages and `url` contain only the origin and path: query
strings, credentials and header values never appear, so logging the error does not leak an API key.

## Target checks

Only absolute `http`/`https` URLs without embedded credentials are accepted. Unless `allowPrivate` is set,
`localhost`, `*.localhost`, `*.internal` and literal loopback, private, link-local (including the cloud
metadata address `169.254.169.254`), carrier-grade NAT and multicast addresses are refused before any
request. The check uses the URL as written: a public hostname that resolves to a private address is not
detected, so keep user-controlled URLs out of `httpCall` or validate them against an allowlist.
