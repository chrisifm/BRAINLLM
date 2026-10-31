# Signed webhook routes

Payment and shipping providers call your backend and sign the exact request bytes. Declare a route with
`verify`, and the backend checks the signature before the handler runs:

```js
// server/routes/payment_events/route.jsbrain
<meta server>
export default {
  method: 'POST',
  path: '/payments/events',
  auth: 'public',
  body: 'raw',
  bodyLimit: '256kb',
  rate: '120/min',
  verify: {
    scheme: 'timestamped',              // header value 't=<unix seconds>,v1=<hex hmac>'
    header: 'x-payment-signature',
    secret: 'PAYMENTS_WEBHOOK_SECRET',  // environment variable NAME, never the value
    tolerance: '5m',                    // default 5m, 1s..1h
  },
}
```

```js
// server/routes/payment_events/handler.jsbrain
<meta server>
export default async function handle(ctx) {
  const event = JSON.parse(ctx.rawBody)
  if (event.type === 'payment.succeeded') {
    // mark the order paid; make this idempotent by event.id, providers retry
  }
  return { received: true }
}
```

## Schemes

| Scheme | Header value | Signed message |
|---|---|---|
| `timestamped` | `t=1767225600,v1=5f2b…` (several `v1` allowed) | `<t>.<raw body>` |
| `hmac` (default) | `sha256=5f2b…` with `prefix: 'sha256='`, or a bare digest | `<raw body>` |

Both use HMAC-SHA256 with a constant-time comparison. `hmac` takes `encoding: 'hex'` (default) or
`'base64'`. `timestamped` rejects a timestamp older or newer than `tolerance`.

## Secrets

`secret` names an environment variable (`.env`, or the host's secret store). Pass a list of up to four
names to rotate: `secret: ['PAYMENTS_WEBHOOK_SECRET', 'PAYMENTS_WEBHOOK_SECRET_PREVIOUS']`. Each secret
must be at least 16 bytes. A missing variable or a literal secret in the contract stops the boot.

## Failures

| Status | Code | Meaning |
|---|---|---|
| `401` | `SIGNATURE_INVALID` | Header missing, malformed or not matching the body |
| `401` | `SIGNATURE_EXPIRED` | `timestamped` signature outside `tolerance` |
| `413` | `BODY_TOO_LARGE` | Body above `bodyLimit` |
| `429` | `RATE_LIMITED` | Above `rate` |

Signed routes skip the browser `Origin` check (providers send none) and must use POST, PUT or PATCH. The
handler receives the bytes as signed in `ctx.rawBody` (`ctx.rawBodyEncoding` is `utf8` or `base64`), the
parsed `ctx.body` when the payload is JSON, and `ctx.verified.timestamp` for the timestamped scheme.

The built-in outbound/inbound webhook endpoints under `/_brain/webhooks` keep their own scheme; signed
routes are for providers that define the URL and signature format themselves.
