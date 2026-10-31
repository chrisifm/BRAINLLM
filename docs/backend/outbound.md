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
| `allowPrivate` | `false` | `true` allows every private target; an array allows only those hostnames (`['billing.internal']`) |
| `redirects` | `0` | Follow up to this many redirects (0..10), checking every hop; `0` returns the 3xx answer |
| `lookup` | `dns.lookup` | Resolver with the `dns.lookup` signature (service discovery, tests); its answers are still checked |
| `signal` | — | `AbortSignal` to cancel the call |

## Retries

Only replayable requests retry: `GET`, `HEAD`, `OPTIONS`, `PUT`, `DELETE`, or any method that carries an
idempotency key. A `POST` without a key is sent once, so a charge is never duplicated by a retry.
Retries happen on network errors, timeouts and statuses 408, 425, 429, 500, 502, 503 and 504. A
`Retry-After` header (seconds or date, capped at 30 s) replaces the backoff delay.

## Result and errors

`httpCall` resolves to `{status, ok, headers, text, json, attempts, url}` (`url` is the redacted final target) for every answer that is not
retryable, including 4xx, so the handler decides what a 404 or 409 means. `json` is `null` unless the
response is JSON. Redirects are returned as answers unless `redirects` is set.

It throws `HttpCallError` with `code` (`TIMEOUT`, `NETWORK`, `HTTP_STATUS`, `TOO_LARGE`, `UNSAFE_URL`,
`REDIRECT`, `ABORTED`), `status`, `attempts` and `url`. Messages and `url` contain only the origin and path: query
strings, credentials and header values never appear, so logging the error does not leak an API key.

## Target checks (SSRF)

Only absolute `http`/`https` URLs without embedded credentials are accepted. Unless the target is allowed, these are
refused with `UNSAFE_URL` before any byte is sent:

- names: `localhost`, `*.localhost`, `*.internal` (including `metadata.google.internal`), `*.local`, `*.home.arpa`;
- IPv4: `0/8`, `10/8`, `127/8`, `169.254/16` (cloud metadata `169.254.169.254`), `172.16/12`, `192.0.0/24`,
  `192.168/16`, `100.64/10` (carrier-grade NAT), `198.18/15`, multicast and reserved (`224/3`);
- IPv6: `::`, `::1`, unique local `fc00::/7` (including `fd00:ec2::254`), link-local, site-local, multicast,
  Teredo, `2001:db8::/32`, and IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible, NAT64 (`64:ff9b::/96`) and 6to4
  (`2002::/16`) addresses whose embedded IPv4 is private, in any spelling the URL parser accepts.

A hostname is resolved by a guarded lookup at connect time and the call is refused if **any** returned address is in
those ranges, so a public name pointing (or rebinding) to an internal address is caught too. Each call opens its own
connection; pooled sockets are never shared between allowed and refused targets.

```js
// server/routes/import/handler.jsbrain
<meta server>
export default async function handle(ctx) {
  // ctx.body.feed comes from a user: redirects are followed, but every hop is checked again
  const feed = await httpCall(ctx.body.feed, { redirects: 3, timeout: 5000, maxBytes: 262144 }).catch(e => e)
  if (feed.code === 'UNSAFE_URL' || feed.code === 'REDIRECT') throw { errors: { feed: 'that address cannot be imported' } }
  if (feed instanceof Error) throw { message: 'the feed did not answer, try again later' }
  // an internal service is named explicitly; other private hosts stay refused
  const stock = await httpCall('http://stock.internal/v1/levels', { allowPrivate: ['stock.internal'] })
  return { status: 200, body: { items: feed.text.length, stock: stock.json } }
}
```

Redirects (`redirects: n`) are re-checked hop by hop: a hop to a private target throws `UNSAFE_URL`, an https→http
downgrade throws `UNSAFE_URL`, more than `n` hops throws `REDIRECT`. A 303, or a 301/302 after `POST`, continues as
`GET` without the body; a hop to another origin drops `authorization`, `cookie` and `proxy-authorization`.

**Development opt-out.** `BRAIN_OUTBOUND_ALLOW_PRIVATE=1` allows private targets in `brain run` and tests (local mocks
of a payment provider). It is ignored when `NODE_ENV` is `production`, which the production server sets, so it
cannot reach a deployed app. In production, name the internal hosts with `allowPrivate: [...]`.
